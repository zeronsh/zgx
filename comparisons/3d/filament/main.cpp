// The Filament arm: the scene drawn by Filament's gltfio (ubershader
// materials, as its viewer) and renderer on Metal into a headless readable
// swap chain, the camera moving along the path every frame.
//
// GPU time is Filament's own (Renderer::getFrameInfoHistory's
// gpuFrameDuration: the frame's command buffers on the GPU). CPU time and
// instructions are the process's (proc_pid_rusage), as for every arm.
#include <filament/Camera.h>
#include <filament/ColorGrading.h>
#include <filament/Engine.h>
#include <filament/IndirectLight.h>
#include <filament/LightManager.h>
#include <filament/Options.h>
#include <filament/Renderer.h>
#include <filament/Scene.h>
#include <filament/SwapChain.h>
#include <filament/Texture.h>
#include <filament/TransformManager.h>
#include <filament/View.h>
#include <filament/Viewport.h>
#include <gltfio/AssetLoader.h>
#include <gltfio/FilamentAsset.h>
#include <gltfio/ResourceLoader.h>
#include <gltfio/TextureProvider.h>
#include <gltfio/materials/uberarchive.h>
#include <ktxreader/Ktx1Reader.h>
#include <utils/EntityManager.h>
#include <utils/NameComponentManager.h>

#include <libproc.h>
#include <mach/mach_time.h>
#include <sys/resource.h>
#include <unistd.h>

#include <algorithm>
#include <numeric>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

using namespace filament;
using namespace filament::math;
using Clock = std::chrono::steady_clock;

static const uint8_t CLEAR[3] = {0x6b, 0x7a, 0x8f};
// Ten seconds at 120 Hz; the footprint is read again after them.
static const int REST_TICKS = 1200;

struct Sample {
    double cpu_ns;
    uint64_t instructions, cycles, footprint, peak;
};
static Sample sample() {
    rusage_info_v4 info{};
    proc_pid_rusage(getpid(), RUSAGE_INFO_V4, (rusage_info_t*)&info);
    mach_timebase_info_data_t base;
    mach_timebase_info(&base);
    return {double(info.ri_user_time + info.ri_system_time) * base.numer / base.denom,
            info.ri_instructions, info.ri_cycles, info.ri_phys_footprint,
            info.ri_lifetime_max_phys_footprint};
}

// A file's bytes in one read (an istreambuf_iterator copy, byte by byte,
// took 1-2 s over Bistro's 406 files and was charged to Filament's load).
static std::vector<uint8_t> slurp(const std::string& path) {
    std::ifstream in(path, std::ios::binary | std::ios::ate);
    std::vector<uint8_t> bytes(size_t(in.tellg()));
    in.seekg(0);
    in.read(reinterpret_cast<char*>(bytes.data()), std::streamsize(bytes.size()));
    return bytes;
}

// The generated instance scene (run.py's `boxes:N:M`): cube i of N on a
// 1.5 m grid about the origin, the first M bobbing half a metre.
static float3 box_at(size_t i, size_t count, size_t frame) {
    size_t side = size_t(std::ceil(std::sqrt(double(count))));
    float half = float(side) * 0.75f;
    float bob = frame > 0 ? 0.5f * std::sin(0.05f * float(frame) + 0.1f * float(i)) : 0.f;
    return {float(i % side) * 1.5f - half, 0.5f + bob, float(i / side) * 1.5f - half};
}
static std::string base64(const std::vector<uint8_t>& bytes) {
    static const char* digits = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    std::string out;
    for (size_t i = 0; i < bytes.size(); i += 3) {
        uint32_t n = uint32_t(bytes[i]) << 16;
        if (i + 1 < bytes.size()) n |= uint32_t(bytes[i + 1]) << 8;
        if (i + 2 < bytes.size()) n |= bytes[i + 2];
        out += digits[(n >> 18) & 63];
        out += digits[(n >> 12) & 63];
        out += i + 1 < bytes.size() ? digits[(n >> 6) & 63] : '=';
        out += i + 2 < bytes.size() ? digits[n & 63] : '=';
    }
    return out;
}
/// A glTF of `count` nodes named b0, b1, … sharing one unit cube and material.
static std::string boxes_gltf(size_t count) {
    std::vector<uint8_t> bin;
    auto put = [&](const void* p, size_t n) { bin.insert(bin.end(), (const uint8_t*)p, (const uint8_t*)p + n); };
    const float faces[6][3] = {{1,0,0},{-1,0,0},{0,1,0},{0,-1,0},{0,0,1},{0,0,-1}};
    std::vector<float> pos, nor;
    std::vector<uint16_t> idx;
    for (auto& f : faces) {
        float3 n{f[0], f[1], f[2]};
        float3 u = std::abs(n.y) > 0.5f ? float3{1, 0, 0} : float3{0, 1, 0};
        float3 v = cross(n, u);
        uint16_t base = uint16_t(pos.size() / 3);
        for (auto [a, b] : {std::pair{-1.f, -1.f}, {1.f, -1.f}, {1.f, 1.f}, {-1.f, 1.f}}) {
            float3 p = 0.5f * (n + a * u + b * v);
            pos.insert(pos.end(), {p.x, p.y, p.z});
            nor.insert(nor.end(), {n.x, n.y, n.z});
        }
        bool flip = dot(cross(u, v), n) < 0;
        if (flip) idx.insert(idx.end(), {base, uint16_t(base + 2), uint16_t(base + 1), base, uint16_t(base + 3), uint16_t(base + 2)});
        else idx.insert(idx.end(), {base, uint16_t(base + 1), uint16_t(base + 2), base, uint16_t(base + 2), uint16_t(base + 3)});
    }
    put(pos.data(), pos.size() * 4); put(nor.data(), nor.size() * 4); put(idx.data(), idx.size() * 2);
    std::string nodes, roots;
    for (size_t i = 0; i < count; i++) {
        float3 p = box_at(i, count, 0);
        char node[160];
        snprintf(node, sizeof node, "%s{\"name\":\"b%zu\",\"mesh\":0,\"translation\":[%g,%g,%g]}", i ? "," : "", i, p.x, p.y, p.z);
        nodes += node;
        roots += (i ? "," : "") + std::to_string(i);
    }
    size_t np = pos.size() * 4, nn = nor.size() * 4, ni = idx.size() * 2;
    char tail[2048];
    snprintf(tail, sizeof tail,
        "],\"meshes\":[{\"primitives\":[{\"attributes\":{\"POSITION\":0,\"NORMAL\":1},\"indices\":2,\"material\":0}]}],"
        "\"materials\":[{\"pbrMetallicRoughness\":{\"baseColorFactor\":[0.578,0.527,0.456,1],\"metallicFactor\":0,\"roughnessFactor\":0.6}}],"
        "\"buffers\":[{\"byteLength\":%zu,\"uri\":\"data:application/octet-stream;base64,%s\"}],"
        "\"bufferViews\":[{\"buffer\":0,\"byteOffset\":0,\"byteLength\":%zu},{\"buffer\":0,\"byteOffset\":%zu,\"byteLength\":%zu},{\"buffer\":0,\"byteOffset\":%zu,\"byteLength\":%zu}],"
        "\"accessors\":[{\"bufferView\":0,\"componentType\":5126,\"count\":24,\"type\":\"VEC3\",\"min\":[-0.5,-0.5,-0.5],\"max\":[0.5,0.5,0.5]},"
        "{\"bufferView\":1,\"componentType\":5126,\"count\":24,\"type\":\"VEC3\"},{\"bufferView\":2,\"componentType\":5123,\"count\":36,\"type\":\"SCALAR\"}]}",
        bin.size(), base64(bin).c_str(), np, np, nn, np + nn, ni);
    return "{\"asset\":{\"version\":\"2.0\"},\"scene\":0,\"scenes\":[{\"nodes\":[" + roots + "]}],\"nodes\":[" + nodes + tail;
}

static std::pair<double, double> spread(std::vector<double> v) {
    if (v.empty()) return {-1, -1};
    std::sort(v.begin(), v.end());
    auto at = [&](double q) { return v[size_t(std::lround((v.size() - 1) * q))]; };
    return {at(0.5), at(0.95)};
}

// A minimal PNG writer (stored deflate blocks), so the arm needs no image library.
static uint32_t crc(const uint8_t* p, size_t n, uint32_t c = 0xffffffff) {
    static uint32_t table[256];
    if (!table[1])
        for (uint32_t i = 0; i < 256; i++) {
            uint32_t r = i;
            for (int k = 0; k < 8; k++) r = r & 1 ? 0xedb88320 ^ (r >> 1) : r >> 1;
            table[i] = r;
        }
    for (size_t i = 0; i < n; i++) c = table[(c ^ p[i]) & 0xff] ^ (c >> 8);
    return c;
}
static void write_png(const std::string& path, uint32_t w, uint32_t h, const std::vector<uint8_t>& rgba) {
    std::vector<uint8_t> raw;
    for (uint32_t y = 0; y < h; y++) {
        raw.push_back(0);
        raw.insert(raw.end(), rgba.begin() + y * w * 4, rgba.begin() + (y + 1) * w * 4);
    }
    std::vector<uint8_t> z = {0x78, 0x01};
    uint32_t a = 1, b = 0;
    for (size_t at = 0; at < raw.size(); at += 65535) {
        size_t n = std::min<size_t>(65535, raw.size() - at);
        z.push_back(at + n == raw.size());
        z.push_back(n & 0xff); z.push_back(n >> 8);
        z.push_back(~n & 0xff); z.push_back((~n >> 8) & 0xff);
        z.insert(z.end(), raw.begin() + at, raw.begin() + at + n);
    }
    for (uint8_t c : raw) { a = (a + c) % 65521; b = (b + a) % 65521; }
    uint32_t adler = (b << 16) | a;
    for (int s = 24; s >= 0; s -= 8) z.push_back(adler >> s);
    std::ofstream out(path, std::ios::binary);
    auto be = [&](uint32_t v) { for (int s = 24; s >= 0; s -= 8) out.put(char(v >> s)); };
    auto chunk = [&](const char* type, const std::vector<uint8_t>& data) {
        be(uint32_t(data.size()));
        std::vector<uint8_t> td(type, type + 4);
        td.insert(td.end(), data.begin(), data.end());
        out.write((const char*)td.data(), td.size());
        be(crc(td.data(), td.size()) ^ 0xffffffff);
    };
    out.write("\x89PNG\r\n\x1a\n", 8);
    std::vector<uint8_t> ihdr;
    for (uint32_t v : {w, h}) for (int s = 24; s >= 0; s -= 8) ihdr.push_back(v >> s);
    ihdr.insert(ihdr.end(), {8, 6, 0, 0, 0});
    chunk("IHDR", ihdr);
    chunk("IDAT", z);
    chunk("IEND", {});
}

int main(int argc, char** argv) {
    auto loadStart = Clock::now();
    std::map<std::string, std::string> args;
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        auto eq = a.find('=');
        if (a.rfind("--", 0) == 0 && eq != std::string::npos) args[a.substr(2, eq - 2)] = a.substr(eq + 1);
    }
    auto floats = [](std::string s) {
        std::replace(s.begin(), s.end(), ',', ' ');
        std::istringstream in(s);
        std::vector<float> v;
        float f;
        while (in >> f) v.push_back(f);
        return v;
    };
    const uint32_t width = std::stoul(args["width"]), height = std::stoul(args["height"]);
    const size_t warmup = std::stoul(args["warmup"]);
    const float fov = args.count("fov") ? std::stof(args["fov"]) : 45.f;
    std::vector<std::pair<float3, float3>> path;
    {
        std::ifstream in(args["path"]);
        std::string line;
        while (std::getline(in, line)) {
            if (line.empty() || line[0] == '#') continue;
            auto v = floats(line);
            path.push_back({{v[0], v[1], v[2]}, {v[3], v[4], v[5]}});
        }
    }
    auto sunv = floats(args["sun"]);
    std::vector<std::string> errors;

    Engine* engine = Engine::create(Engine::Backend::METAL);
    SwapChain* swap = engine->createSwapChain(width, height, SwapChain::CONFIG_READABLE);
    Renderer* renderer = engine->createRenderer();
    Scene* scene = engine->createScene();
    View* view = engine->createView();
    utils::Entity cameraEntity = utils::EntityManager::get().create();
    Camera* camera = engine->createCamera(cameraEntity);
    view->setScene(scene);
    view->setCamera(camera);
    view->setViewport({0, 0, width, height});
    // As the other arms: 4x MSAA and nothing else post-processed but the
    // tone map (Filament's defaults add FXAA and dithering).
    view->setAntiAliasing(View::AntiAliasing::NONE);
    view->setDithering(View::Dithering::NONE);
    MultiSampleAntiAliasingOptions msaa;
    uint8_t samples = args.count("msaa") ? uint8_t(std::stoul(args["msaa"])) : 4;
    msaa.enabled = samples > 1;
    msaa.sampleCount = samples;
    view->setMultiSampleAntiAliasingOptions(msaa);
    view->setShadowType(View::ShadowType::PCF);
    // `--post=bloom,lut`: the post column. Filament grades through a 3D
    // table it bakes from the grading's parameters, in every arm (ACES
    // included), so the post arm's grade costs the table it always reads;
    // bloom at Filament's default strength.
    std::string post = args.count("post") ? args["post"] : "";
    auto postOn = [&](const char* part) { return ("," + post + ",").find(std::string(",") + part + ",") != std::string::npos; };
    ColorGrading::Builder gradingBuilder;
    gradingBuilder.toneMapping(ColorGrading::ToneMapping::ACES);
    if (postOn("lut")) {
        gradingBuilder.whiteBalance(0.1f, 0.f).contrast(1.05f);
    }
    ColorGrading* grading = gradingBuilder.build(*engine);
    view->setColorGrading(grading);
    if (postOn("bloom")) {
        View::BloomOptions bloom;
        bloom.enabled = true;
        view->setBloomOptions(bloom);
    }
    Renderer::ClearOptions clear;
    clear.clearColor = {std::pow(CLEAR[0] / 255.f, 2.2f), std::pow(CLEAR[1] / 255.f, 2.2f),
                        std::pow(CLEAR[2] / 255.f, 2.2f), 1.f};
    clear.clear = true;
    renderer->setClearOptions(clear);
    // Exposure 1: EV100 = log2(N² / t · 100 / S) = log2(1 / 1.2).
    camera->setExposure(1.f, 1.2f, 100.f);
    camera->setProjection(fov, double(width) / height, 0.1, 300.0, Camera::Fov::VERTICAL);

    // The environment, prefiltered offline by cmgen (fetch.py).
    auto envStart = Clock::now();
    std::string iblPath;
    for (auto& entry : std::filesystem::directory_iterator(args["ibl"]))
        if (entry.path().string().find("_ibl.ktx") != std::string::npos) iblPath = entry.path();
    auto iblBytes = slurp(iblPath);
    auto* bundle = new image::Ktx1Bundle(iblBytes.data(), uint32_t(iblBytes.size()));
    float3 sh[9];
    bundle->getSphericalHarmonics(sh);
    Texture* ibl = ktxreader::Ktx1Reader::createTexture(engine, bundle, false);
    IndirectLight* indirect = IndirectLight::Builder()
        .reflections(ibl)
        .irradiance(3, sh)
        .intensity(1.f)
        .build(*engine);
    scene->setIndirectLight(indirect);
    double environment_ms = std::chrono::duration<double, std::milli>(Clock::now() - envStart).count();

    utils::Entity sun = utils::EntityManager::get().create();
    LightManager::ShadowOptions shadows;
    shadows.mapSize = 2048;
    shadows.shadowCascades = 1;
    LightManager::Builder(LightManager::Type::DIRECTIONAL)
        .color({1.f, 0.96f, 0.9f})
        .intensity(3.f)  // lux, diffuse over π, exposure 1: as zgx
        .direction(normalize(float3{sunv[0], sunv[1], sunv[2]}))
        .castShadows(true)
        .shadowOptions(shadows)
        .build(*engine, sun);
    scene->addEntity(sun);

    // `--point-lights=N`: the light scene, N shadowless point lights
    // (Filament's froxelised forward), placed as common/report.rs's light_at.
    size_t pointLights = args.count("point-lights") ? std::stoul(args["point-lights"]) : 0;
    for (size_t i = 0; i < pointLights; i++) {
        uint32_t state = uint32_t(i) * 2654435761u + 12345u;
        auto next = [&] {
            state ^= state << 13; state ^= state >> 17; state ^= state << 5;
            return float(state % 10000) / 10000.f;
        };
        float3 p{-40.f + 130.f * next(), 1.f + 5.f * next(), -60.f + 130.f * next()};
        float warm = next();
        utils::Entity light = utils::EntityManager::get().create();
        LightManager::Builder(LightManager::Type::POINT)
            .color({1.f, 0.6f + 0.4f * (1.f - warm), 0.4f + 0.6f * warm})
            .intensity(150.f)
            .falloff(8.f)
            .position(p)
            .build(*engine, light);
        scene->addEntity(light);
    }

    // The model, by gltfio as its viewer loads one.
    auto modelStart = Clock::now();
    std::filesystem::path modelPath = args["model"];
    size_t boxCount = 0, boxMoving = 0;
    std::vector<uint8_t> gltf;
    if (sscanf(args["model"].c_str(), "boxes:%zu:%zu", &boxCount, &boxMoving) == 2) {
        std::string text = boxes_gltf(boxCount);
        gltf.assign(text.begin(), text.end());
        modelPath = "boxes.gltf";
    } else {
        boxCount = 0;
        gltf = slurp(modelPath);
    }
    auto* materials = gltfio::createUbershaderProvider(engine, UBERARCHIVE_DEFAULT_DATA,
                                                        UBERARCHIVE_DEFAULT_SIZE);
    utils::NameComponentManager names(utils::EntityManager::get());
    gltfio::AssetLoader* loader = gltfio::AssetLoader::create({engine, materials, &names});
    auto parseStart = Clock::now();
    gltfio::FilamentAsset* asset = loader->createAsset(gltf.data(), uint32_t(gltf.size()));
    double parse_ms = std::chrono::duration<double, std::milli>(Clock::now() - parseStart).count();
    auto readStart = Clock::now();
    if (!asset) { fprintf(stderr, "the model didn't load\n"); return 1; }
    gltfio::ResourceConfiguration config{};
    config.engine = engine;
    config.normalizeSkinningWeights = true;
    // Deprecated in 1.80 but still how gltfio resolves a URI to its cache entry.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    std::string gltfPath = modelPath.string();
    config.gltfPath = gltfPath.c_str();
#pragma clang diagnostic pop
    gltfio::ResourceLoader resources(config);
    auto* stb = gltfio::createStbProvider(engine);
    resources.addTextureProvider("image/png", stb);
    resources.addTextureProvider("image/jpeg", stb);
    for (size_t i = 0; i < asset->getResourceUriCount(); i++) {
        const char* uri = asset->getResourceUris()[i];
        auto* bytes = new std::vector<uint8_t>(slurp((modelPath.parent_path() / uri).string()));
        resources.addResourceData(uri, {bytes->data(), bytes->size(),
            [](void*, size_t, void* user) { delete (std::vector<uint8_t>*)user; }, bytes});
    }
    double read_ms = std::chrono::duration<double, std::milli>(Clock::now() - readStart).count();
    fprintf(stderr, "%zu resource URIs\n", asset->getResourceUriCount());
    auto resourcesStart = Clock::now();
    if (!resources.loadResources(asset)) {
        fprintf(stderr, "the model's resources didn't load\n");
        return 1;
    }
    double resources_ms = std::chrono::duration<double, std::milli>(Clock::now() - resourcesStart).count();
    asset->releaseSourceData();
    scene->addEntities(asset->getEntities(), asset->getEntityCount());
    // The moving cubes' transforms, by their nodes' names.
    std::vector<TransformManager::Instance> movers(boxMoving);
    auto& transforms = engine->getTransformManager();
    if (boxMoving) {
        for (size_t e = 0; e < asset->getEntityCount(); e++) {
            const char* name = names.getName(names.getInstance(asset->getEntities()[e]));
            size_t i;
            if (name && sscanf(name, "b%zu", &i) == 1 && i < boxMoving)
                movers[i] = transforms.getInstance(asset->getEntities()[e]);
        }
    }
    auto move = [&](size_t frame) {
        for (size_t i = 0; i < boxMoving; i++)
            transforms.setTransform(movers[i], mat4f::translation(box_at(i, boxCount, frame)));
    };
    double model_ms = std::chrono::duration<double, std::milli>(Clock::now() - modelStart).count();

    auto pose = [&](size_t i) {
        auto [p, t] = path[std::min(i, path.size() - 1)];
        camera->lookAt(double3(p), double3(t), {0, 1, 0});
    };

    // Draws a frame; true if Filament drew it (beginFrame may skip one).
    // Filament skips a frame (beginFrame false) while the GPU is too far
    // behind; a vsync-paced app would wait for the display, so this waits a
    // little and asks again, counting each time it was refused.
    size_t refused = 0;
    auto frame = [&](std::vector<uint8_t>* read) {
        while (!renderer->beginFrame(swap)) {
            refused++;
            std::this_thread::sleep_for(std::chrono::microseconds(50));
        }
        renderer->render(view);
        if (read) {
            read->resize(size_t(width) * height * 4);
            renderer->readPixels(0, 0, width, height,
                {read->data(), read->size(), backend::PixelDataFormat::RGBA,
                 backend::PixelDataType::UBYTE});
        }
        renderer->endFrame();
        return true;
    };

    // Warm-up: pipelines and textures made, at the first pose.
    pose(0);
    size_t skipped = 0;
    // Load: from the process's start to the first frame drawn.
    auto firstStart = Clock::now();
    frame(nullptr);
    engine->flushAndWait();
    double first_ms = std::chrono::duration<double, std::milli>(Clock::now() - firstStart).count();
    double load_ms = std::chrono::duration<double, std::milli>(Clock::now() - loadStart).count();
    uint64_t loadPeak = sample().peak;
    for (size_t i = 1; i < warmup; i++) skipped += !frame(nullptr);
    engine->flushAndWait();

    std::vector<double> frame_ms, submit_ms, gpu_ms;
    std::map<uint32_t, double> gpu;
    uint32_t firstId = renderer->getFrameInfoHistory(1).empty()
        ? 0 : renderer->getFrameInfoHistory(1)[0].frameId + 1;
    Sample before = sample();
    auto from = Clock::now(), last = from;
    size_t drawn = 0;
    for (size_t i = 0; i < path.size(); i++) {
        auto start = Clock::now();
        pose(i);
        move(warmup + i);
        if (frame(nullptr)) drawn++; else skipped++;
        auto now = Clock::now();
        submit_ms.push_back(std::chrono::duration<double, std::milli>(now - start).count());
        frame_ms.push_back(std::chrono::duration<double, std::milli>(now - last).count());
        last = now;
        // The GPU times of frames done so far (the history keeps a few, each
        // complete a frame or two after it was drawn).
        for (auto& info : renderer->getFrameInfoHistory(renderer->getMaxFrameHistorySize()))
            if (info.gpuFrameDuration > 0) gpu[info.frameId] = info.gpuFrameDuration / 1e6;
    }
    engine->flushAndWait();
    double elapsed = std::chrono::duration<double>(Clock::now() - from).count();
    Sample after = sample();
    for (auto& info : renderer->getFrameInfoHistory(renderer->getMaxFrameHistorySize()))
        if (info.gpuFrameDuration > 0) gpu[info.frameId] = info.gpuFrameDuration / 1e6;
    for (auto [id, ms] : gpu)
        if (id >= firstId) gpu_ms.push_back(ms);
    if (gpu_ms.size() < path.size() / 2)
        errors.push_back("GPU durations for only " + std::to_string(gpu_ms.size()) + " frames");

    // At rest: an app loop ticking at 120 Hz, nothing moving. Filament draws
    // whenever the app calls render; this loop calls it every tick, as its
    // samples do.
    Sample restBefore = sample();
    size_t restFrames = 0;
    for (int i = 0; i < REST_TICKS; i++) {
        auto start = Clock::now();
        restFrames += frame(nullptr);
        std::this_thread::sleep_until(start + std::chrono::microseconds(8333));
    }
    engine->flushAndWait();
    Sample restAfter = sample();

    // The last pose's pixels.
    pose(path.size() - 1);
    std::vector<uint8_t> rgba;
    while (!frame(&rgba)) {}
    engine->flushAndWait();
    // On Metal, readPixels' rows already run top to bottom.
    std::vector<uint8_t>& flipped = rgba;
    write_png(args["out"], width, height, flipped);
    uint64_t hash = 0xcbf29ce484222325;
    size_t differ = 0;
    for (size_t i = 0; i < flipped.size(); i += 4) {
        for (int c = 0; c < 4; c++) hash = (hash ^ flipped[i + c]) * 0x100000001b3;
        if (std::abs(flipped[i] - CLEAR[0]) > 2 || std::abs(flipped[i + 1] - CLEAR[1]) > 2 ||
            std::abs(flipped[i + 2] - CLEAR[2]) > 2)
            differ++;
    }
    if (skipped) errors.push_back(std::to_string(skipped) + " frames skipped by beginFrame");

    auto [gpu_med, gpu_p95] = spread(gpu_ms);
    auto [frame_med, frame_p95] = spread(frame_ms);
    auto [submit_med, submit_p95] = spread(submit_ms);
    double n = double(path.size());
    std::string err;
    for (auto& e : errors) err += (err.empty() ? "" : "; ") + e;
    printf("RESULT {\"arm\": \"filament\", \"adapter\": \"Metal (Filament v1.80.0)\", "
           "\"frames\": %zu, \"environment_ms\": %f, \"model_ms\": %f, "
           "\"gpu_ms_per_frame\": %f, \"gpu_ms_median\": %f, \"gpu_ms_p95\": %f, \"gpu_samples\": %zu, "
           "\"frame_ms_median\": %f, \"frame_ms_p95\": %f, "
           "\"submit_ms_median\": %f, \"submit_ms_p95\": %f, \"fps\": %f, "
           "\"cpu_ms_per_frame\": %f, \"instructions_per_frame\": %f, \"cycles_per_frame\": %f, "
           "\"footprint_mb\": %f, \"peak_footprint_mb\": %f, \"hash\": %llu, \"drawn_fraction\": %f, "
           "\"begin_refused\": %zu, \"load_phases_ms\": \"IBL (offline-prefiltered KTX) %.0f; createAsset (parse) %.0f; files read %.0f; loadResources (decode, upload, mips) %.0f; first frame %.0f\", \"load_ms\": %f, \"load_peak_footprint_mb\": %f, "
           "\"rested_footprint_mb\": %f, \"rest_seconds\": 10, \"rest_ticks\": %d, \"rest_frames\": %zu, \"rest_cpu_ms_per_tick\": %f, \"errors\": \"%s\"}\n",
           drawn, environment_ms, model_ms,
           gpu_ms.empty() ? -1. : std::accumulate(gpu_ms.begin(), gpu_ms.end(), 0.) / double(gpu_ms.size()), gpu_med, gpu_p95, gpu_ms.size(), frame_med, frame_p95,
           submit_med, submit_p95, n / elapsed, (after.cpu_ns - before.cpu_ns) / 1e6 / n,
           double(after.instructions - before.instructions) / n,
           double(after.cycles - before.cycles) / n, after.footprint / 1048576., after.peak / 1048576.,
           (unsigned long long)hash, double(differ) / (width * height), refused, environment_ms, parse_ms, read_ms, resources_ms, first_ms, load_ms, loadPeak / 1048576., restAfter.footprint / 1048576., REST_TICKS, restFrames,
           (restAfter.cpu_ns - restBefore.cpu_ns) / 1e6 / REST_TICKS, err.c_str());
    fflush(stdout);
    std::_Exit(0);  // the measurement is out; skip tearing the engine down
}
