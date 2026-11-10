#!/usr/bin/env python3
"""Make Amazon Lumberyard Bistro (fetched by fetch.py) a glTF every arm reads.

Bistro is CC-BY 4.0: "Amazon Lumberyard Bistro, Open Research Content
Archive (ORCA)", Amazon Lumberyard, 2017,
http://developer.nvidia.com/orca/amazon-lumberyard-bistro. It ships as FBX
with DDS textures, which no arm reads, so this:

1. converts the FBX with assimp (`assimp export … -f gltf2`; run, never
   linked; its version is recorded in the output's `asset.generator`);
2. rewrites each material as Bistro's README describes its textures:
   BaseColor (alpha: opacity), Specular (R occlusion, G roughness,
   B metalness: glTF's metallicRoughness layout, used as that; its
   occlusion is left out, as Bistro's placeholder maps hold 0 there and
   would take all ambient light from whatever they cover), Normal (DirectX: green flipped for
   glTF), Emissive. assimp's KHR_materials_specular reading of the
   Specular map is dropped. A base colour whose alpha cuts holes is
   alphaMode MASK at 0.5;
3. writes each DDS once as a PNG, at most `--max-side` texels a side
   (1024 by default: no arm compresses PNG textures on the GPU, and at
   Bistro's 2048² every arm would hold several GB of RGBA8).

The output is the same input for every engine. Needs assimp on PATH and
Pillow (for DDS: DXT1/5, BC4/5/7).

    python3 comparisons/3d/convert_bistro.py SRC_DIR OUT_DIR [--scene exterior]
"""
import argparse
import json
import os
import re
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

from PIL import Image

ATTRIBUTION = ("Amazon Lumberyard Bistro, Open Research Content Archive (ORCA), "
               "Amazon Lumberyard, 2017, CC-BY 4.0, "
               "http://developer.nvidia.com/orca/amazon-lumberyard-bistro")
FBX = {"exterior": "BistroExterior.fbx", "interior": "BistroInterior.fbx"}


def assimp_version():
    out = subprocess.run(["assimp", "version"], capture_output=True, text=True).stdout
    found = re.search(r"Version (\d+\.\d+(?:\.\d+)?)", out)
    return found.group(1) if found else "unknown"


def convert_image(args):
    """One DDS to PNG: (out name, has cut-out alpha). `kind` picks the fix."""
    src, dst, kind, max_side = args
    image = Image.open(src)
    image.load()
    if max(image.size) > max_side:
        scale = max_side / max(image.size)
        size = (max(1, round(image.size[0] * scale)), max(1, round(image.size[1] * scale)))
        image = image.resize(size, Image.LANCZOS)
    cut = False
    if kind == "normal":
        image = image.convert("RGB")
        r, g, b = image.split()
        g = g.point(lambda v: 255 - v)  # DirectX to glTF (OpenGL) green
        if image.getextrema()[2] == (0, 0):  # BC5 holds X and Y alone
            b = Image.new("L", image.size, 255)
        image = Image.merge("RGB", (r, g, b))
    elif kind == "base" and image.mode == "RGBA":
        alpha = image.getchannel("A")
        histogram = alpha.histogram()
        cut = sum(histogram[:128]) > 0.001 * image.size[0] * image.size[1]
        if not cut:
            image = image.convert("RGB")
    elif image.mode not in ("RGB", "RGBA", "L"):
        image = image.convert("RGBA")
    dst.parent.mkdir(parents=True, exist_ok=True)
    image.save(dst, optimize=False, compress_level=6)
    return dst.name, cut


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("src", type=Path, help="the unzipped Bistro_v5_2 folder")
    parser.add_argument("out", type=Path)
    parser.add_argument("--scene", default="exterior", choices=FBX)
    parser.add_argument("--max-side", type=int, default=1024)
    args = parser.parse_args()
    out = args.out / args.scene
    out.mkdir(parents=True, exist_ok=True)
    raw = out / f"{args.scene}.assimp.gltf"
    if not raw.exists():
        subprocess.run(["assimp", "export", str(args.src / FBX[args.scene]), str(raw), "-f", "gltf2"],
                       check=True)
    doc = json.loads(raw.read_text())

    # Each texture's image by Bistro's file names.
    def image_of(texture_index):
        uri = doc["images"][doc["textures"][texture_index]["source"]]["uri"]
        return uri.replace("\\", "/").split("/")[-1]

    jobs = {}  # source name -> (png name, kind)
    plans = []
    for material in doc["materials"]:
        pbr = material.get("pbrMetallicRoughness", {})
        names = {}
        if "baseColorTexture" in pbr:
            names["base"] = image_of(pbr["baseColorTexture"]["index"])
        if "normalTexture" in material:
            names["normal"] = image_of(material["normalTexture"]["index"])
        if "emissiveTexture" in material:
            names["emissive"] = image_of(material["emissiveTexture"]["index"])
        specular = material.get("extensions", {}).get("KHR_materials_specular", {})
        if "specularTexture" in specular:
            names["orm"] = image_of(specular["specularTexture"]["index"])
        elif "base" in names and names["base"].endswith("_BaseColor.dds"):
            guess = names["base"].replace("_BaseColor.dds", "_Specular.dds")
            if (args.src / "Textures" / guess).exists():
                names["orm"] = guess
        for kind, name in names.items():
            if (args.src / "Textures" / name).exists():
                jobs.setdefault((name, kind), f"{Path(name).stem}{'_n' if kind == 'normal' else ''}.png")
        plans.append((material, names))

    work = [(args.src / "Textures" / name, out / "textures" / png, kind, args.max_side)
            for (name, kind), png in jobs.items()]
    cut_outs = set()
    with ProcessPoolExecutor(max_workers=max(1, (os.cpu_count() or 4) // 2)) as pool:
        for (name, kind), (png, cut) in zip(jobs, pool.map(convert_image, work, chunksize=4)):
            if cut:
                cut_outs.add(name)
    print(f"{len(work)} textures written", file=sys.stderr)

    images, textures, index = [], [], {}

    def texture(name, kind):
        png = jobs.get((name, kind))
        if png is None:
            return None
        if png not in index:
            index[png] = len(textures)
            images.append({"uri": f"textures/{png}"})
            textures.append({"source": len(images) - 1, "sampler": 0})
        return {"index": index[png]}

    for material, names in plans:
        material.pop("extensions", None)
        pbr = {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 1.0, "roughnessFactor": 1.0}
        old = material.get("pbrMetallicRoughness", {})
        if "base" in names and (t := texture(names["base"], "base")):
            pbr["baseColorTexture"] = t
        elif "baseColorFactor" in old:
            pbr["baseColorFactor"] = old["baseColorFactor"]
        if "orm" in names and (t := texture(names["orm"], "orm")):
            pbr["metallicRoughnessTexture"] = t
        else:
            pbr["metallicFactor"], pbr["roughnessFactor"] = 0.0, 0.6
        material["pbrMetallicRoughness"] = pbr
        if "normal" in names and (t := texture(names["normal"], "normal")):
            material["normalTexture"] = t
        else:
            material.pop("normalTexture", None)
        if "emissive" in names and (t := texture(names["emissive"], "emissive")):
            material["emissiveTexture"] = t
            material["emissiveFactor"] = [1, 1, 1]
        else:
            material.pop("emissiveTexture", None)
            material.pop("emissiveFactor", None)
        material.pop("occlusionTexture", None)
        if names.get("base") in cut_outs:
            material["alphaMode"], material["alphaCutoff"] = "MASK", 0.5
            material["doubleSided"] = True
    doc["images"], doc["textures"] = images, textures
    doc["samplers"] = [{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]
    doc["extensionsUsed"] = [e for e in doc.get("extensionsUsed", [])
                             if e not in ("KHR_materials_specular", "KHR_materials_volume", "FB_ngon_encoding")]
    if not doc["extensionsUsed"]:
        doc.pop("extensionsUsed")
    doc["asset"]["generator"] = f"assimp {assimp_version()} + zgx comparisons/3d/convert_bistro.py"
    doc["asset"]["copyright"] = ATTRIBUTION
    buffers = doc["buffers"]
    for buffer in buffers:
        buffer["uri"] = buffer["uri"].replace(f"{args.scene}.assimp", args.scene)
    (out / f"{args.scene}.gltf").write_text(json.dumps(doc))
    raw_bin = out / f"{args.scene}.assimp.bin"
    if raw_bin.exists():
        raw_bin.replace(out / f"{args.scene}.bin")
    print(f"wrote {out / (args.scene + '.gltf')}: {len(doc['materials'])} materials, "
          f"{len(textures)} textures, {len(cut_outs)} cut-out", file=sys.stderr)


if __name__ == "__main__":
    main()
