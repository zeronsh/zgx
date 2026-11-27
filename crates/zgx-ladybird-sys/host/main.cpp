/*
 * zgx's Ladybird host: runs Ladybird's LibWebView for one zgx app, which talks to it over the socket it passes
 * (--zgx-socket <fd>) in the messages of Protocol.h. Each of the app's web views is a view here; the frames Ladybird's
 * Compositor presents go to the app as the shared buffers they are in (their descriptors once, then which one shows
 * what), and nothing is sent while a page is idle.
 */

#include "Protocol.h"

#include <AK/HashMap.h>
#include <AK/LexicalPath.h>
#include <AK/OwnPtr.h>
#include <LibCore/AnonymousBuffer.h>
#include <LibCore/ArgsParser.h>
#include <LibCore/EventLoop.h>
#include <LibCore/Notifier.h>
#include <LibCore/System.h>
#include <LibGfx/Bitmap.h>
#include <LibGfx/Cursor.h>
#include <LibGfx/SharedImageBuffer.h>
#include <LibGfx/SystemTheme.h>
#include <LibMain/Main.h>
#include <LibURL/Parser.h>
#include <LibWebCommon/Page/InputEvent.h>
#include <LibWebCommon/WebView/Utilities.h>
#include <LibWebView/Application.h>
#include <LibWebView/HeadlessWebView.h>
#include <LibWebView/PageBackingStores.h>
#include <LibWebCommon/Clipboard/SystemClipboard.h>
#include <LibWebView/WebContentPage.h>

#include <errno.h>
#include <sys/socket.h>
#include <unistd.h>

namespace ZgxHost {

class Connection;
static Connection* s_connection;

class Connection {
public:
    explicit Connection(int fd)
        : m_fd(fd)
    {
        m_notifier = Core::Notifier::construct(fd, Core::Notifier::Type::Read);
        m_notifier->on_activation = [this] { read_available(); };
    }

    Function<void(ToHost, Reader&)> on_message;
    Function<void()> on_closed;

    void send(Writer& writer, Optional<int> fd = {})
    {
        auto const& bytes = writer.finish();
        size_t sent = 0;
        while (sent < bytes.size()) {
            struct iovec iov { const_cast<u8*>(bytes.data() + sent), bytes.size() - sent };
            struct msghdr message {};
            message.msg_iov = &iov;
            message.msg_iovlen = 1;
            alignas(struct cmsghdr) char control[CMSG_SPACE(sizeof(int))] {};
            if (sent == 0 && fd.has_value()) {
                message.msg_control = control;
                message.msg_controllen = sizeof(control);
                auto* header = CMSG_FIRSTHDR(&message);
                header->cmsg_level = SOL_SOCKET;
                header->cmsg_type = SCM_RIGHTS;
                header->cmsg_len = CMSG_LEN(sizeof(int));
                __builtin_memcpy(CMSG_DATA(header), &*fd, sizeof(int));
            }
            auto n = ::sendmsg(m_fd, &message, 0);
            if (n < 0) {
                if (errno == EINTR)
                    continue;
                // The app is gone: nothing more to show anything to.
                if (on_closed)
                    on_closed();
                return;
            }
            sent += n;
        }
    }

private:
    void read_available()
    {
        u8 buffer[64 * 1024];
        auto n = ::read(m_fd, buffer, sizeof(buffer));
        if (n <= 0) {
            if (n < 0 && (errno == EINTR || errno == EAGAIN))
                return;
            m_notifier->set_enabled(false);
            if (on_closed)
                on_closed();
            return;
        }
        m_pending.append(buffer, n);
        size_t offset = 0;
        while (m_pending.size() - offset >= 5) {
            u32 length;
            __builtin_memcpy(&length, m_pending.data() + offset, 4);
            if (m_pending.size() - offset - 4 < length)
                break;
            auto kind = static_cast<ToHost>(m_pending[offset + 4]);
            Reader reader { ReadonlyBytes { m_pending.data() + offset + 5, length - 1 } };
            if (on_message)
                on_message(kind, reader);
            offset += 4 + length;
        }
        m_pending.remove(0, offset);
    }

    int m_fd { -1 };
    RefPtr<Core::Notifier> m_notifier;
    Vector<u8> m_pending;
};

// One of the app's web views.
class View final : public WebView::HeadlessWebView {
public:
    static NonnullOwnPtr<View> create(u64 id, Core::AnonymousBuffer theme, Web::DevicePixelSize size, double device_pixel_ratio)
    {
        auto view = adopt_own(*new View(id, move(theme), size));
        view->m_device_pixel_ratio = device_pixel_ratio;
        view->initialize_tab(Web::HTML::VisibilityState::Visible);
#if !defined(AK_OS_MACOS)
        // Frames in shared memory, which the app maps and uploads what changed of: a GPU's buffers would need
        // importing into the app's device, which comes later.
        view->set_presents_from_cpu_memory(true);
#endif
        return view;
    }

    void resize(Web::DevicePixelSize size, double device_pixel_ratio)
    {
        m_device_pixel_ratio = device_pixel_ratio;
        reset_viewport_size(size);
        handle_resize();
    }

    // An editing command from the app: 0 copy, 1 cut, 2 paste, 3 select all, 4 undo, 5 redo. Copy and cut put the
    // selection on the app's clipboard; paste reads it (the Application's clipboard hooks, below).
    void edit(u8 command)
    {
        auto to_clipboard = [](ByteString const& text) -> ErrorOr<void> {
            WebView::Application::the().set_clipboard_text(MUST(String::from_byte_string(text)));
            return {};
        };
        switch (command) {
        case 0:
            selected_text()->when_resolved(to_clipboard);
            break;
        case 1:
            cut_selected_text()->when_resolved(to_clipboard);
            break;
        case 2:
            paste_from_clipboard(Web::Clipboard::PasteMode::AllMimeTypes);
            break;
        case 3:
            select_all();
            break;
        case 4:
            undo();
            break;
        case 5:
            redo();
            break;
        default:
            break;
        }
    }

    // The whole document's pixels, painted by WebContent for this, to the app as a shared buffer (`ToZgx::Snapshot`).
    // One at a time, as Ladybird takes them; a request while one is out is answered at once without one.
    void snapshot(u64 request)
    {
        if (m_snapshot_request.has_value()) {
            send_snapshot(request, nullptr);
            return;
        }
        m_snapshot_request = request;
        page().async_take_document_screenshot();
    }

    // The app is done reading the frame it was last sent, up to `bitmap_id`'s: every backing store Ladybird let go
    // of before it may be painted into again.
    void done(i32 bitmap_id)
    {
        if (bitmap_id != m_last_sent_bitmap)
            return;
        for (auto id : m_held_releases)
            WebView::HeadlessWebView::release_backing_store(id);
        m_held_releases.clear();
    }

private:
    View(u64 id, Core::AnonymousBuffer theme, Web::DevicePixelSize size)
        : WebView::HeadlessWebView(move(theme), size)
        , m_id(id)
    {
        on_title_change = [this](Utf16String const& title) {
            auto text = title.to_utf8();
            Writer writer { ToZgx::Title };
            writer.u64_(m_id).str(text.bytes_as_string_view());
            s_connection->send(writer);
        };
        on_url_change = [this](URL::URL const& url) {
            auto text = url.serialize();
            Writer writer { ToZgx::Url };
            writer.u64_(m_id).str(text.bytes_as_string_view());
            s_connection->send(writer);
            send_history();
        };
        on_loading_state_change = [this](bool loading) {
            Writer writer { ToZgx::Loading };
            writer.u64_(m_id).u8_(loading ? 1 : 0);
            s_connection->send(writer);
        };
        on_load_finish = [this](URL::URL const& url) {
            auto text = url.serialize();
            Writer writer { ToZgx::LoadFinished };
            writer.u64_(m_id).str(text.bytes_as_string_view());
            s_connection->send(writer);
            send_history();
        };
        on_link_hover = [this](URL::URL const& url) {
            auto text = url.serialize();
            Writer writer { ToZgx::LinkHovered };
            writer.u64_(m_id).str(text.bytes_as_string_view());
            s_connection->send(writer);
        };
        on_link_unhover = [this] {
            Writer writer { ToZgx::LinkHovered };
            writer.u64_(m_id).str(""sv);
            s_connection->send(writer);
        };
        // A page without a theme colour reports a transparent one.
        on_theme_color_change = [this](Gfx::Color color) {
            Writer writer { ToZgx::ThemeColor };
            writer.u64_(m_id).u8_(color.alpha() > 0 ? 1 : 0).u8_(color.red()).u8_(color.green()).u8_(color.blue()).u8_(color.alpha());
            s_connection->send(writer);
        };
        // Why WebContent ended: 0 crashed, 1 out of memory, 2 killed (it broke the IPC protocol).
        on_web_content_crashed = [this](WebContentCrashReason reason) {
            u8 why = 0;
            if (reason == WebContentCrashReason::ExceededMemoryLimit)
                why = 1;
            else if (reason == WebContentCrashReason::RejectedIPC)
                why = 2;
            Writer writer { ToZgx::Crashed };
            writer.u64_(m_id).u8_(why);
            s_connection->send(writer);
        };
        // The page's icon, its pixels inline (icons are small); none when it has none.
        on_favicon_change = [this](Optional<Gfx::Bitmap const&> favicon) {
            Writer writer { ToZgx::Favicon };
            writer.u64_(m_id);
            if (!favicon.has_value()) {
                writer.u8_(0);
            } else {
                auto const& bitmap = *favicon;
                writer.u8_(1)
                    .u32_(bitmap.width())
                    .u32_(bitmap.height())
                    .u32_(bitmap.pitch())
                    .u8_(static_cast<u8>(bitmap.format()))
                    .u8_(bitmap.alpha_type() == Gfx::AlphaType::Premultiplied ? 1 : 0)
                    .bytes({ bitmap.scanline_u8(0), bitmap.size_in_bytes() });
            }
            s_connection->send(writer);
        };
        on_cursor_change = [this](Gfx::Cursor const& cursor) {
            u8 standard = 0;
            if (auto const* named = cursor.get_pointer<Gfx::StandardCursor>())
                standard = static_cast<u8>(*named);
            Writer writer { ToZgx::Cursor };
            writer.u64_(m_id).u8_(standard);
            s_connection->send(writer);
        };
    }

    void send_snapshot(u64 request, Gfx::Bitmap const* bitmap)
    {
        Writer writer { ToZgx::Snapshot };
        writer.u64_(m_id).u64_(request);
        if (!bitmap || !bitmap->anonymous_buffer().is_valid()) {
            writer.u8_(0);
            s_connection->send(writer);
            return;
        }
        auto const& buffer = bitmap->anonymous_buffer();
        writer.u8_(1)
            .u32_(bitmap->width())
            .u32_(bitmap->height())
            .u32_(bitmap->pitch())
            .u8_(static_cast<u8>(bitmap->format()))
            .u8_(bitmap->alpha_type() == Gfx::AlphaType::Premultiplied ? 1 : 0)
            .u64_(buffer.size());
        s_connection->send(writer, buffer.fd());
    }

    // The host asks for no screenshots of Ladybird's own (they save files), so one it didn't ask for is dropped.
    virtual void did_receive_screenshot(Badge<WebView::WebContentPage>, Gfx::ShareableBitmap const& screenshot) override
    {
        if (!m_snapshot_request.has_value())
            return;
        auto request = m_snapshot_request.release_value();
        send_snapshot(request, screenshot.is_valid() ? screenshot.bitmap() : nullptr);
    }

    void send_history()
    {
        Writer writer { ToZgx::History };
        writer.u64_(m_id).u8_(can_navigate_back() ? 1 : 0).u8_(can_navigate_forward() ? 1 : 0);
        s_connection->send(writer);
    }

    virtual void did_change_session_history(WebView::SessionHistorySnapshot const&, URL::URL const&) override { send_history(); }

    virtual void did_accept_presented_backing_store(i32 bitmap_id, Gfx::IntRect damage) override
    {
        auto frame = backing_stores().displayed_frame();
        if (!frame.has_value() || !frame->shared_image_buffer)
            return;
#if !defined(AK_OS_MACOS)
        auto bitmap = frame->shared_image_buffer->bitmap_if_present();
        if (!bitmap)
            return;
        auto& buffer = bitmap->anonymous_buffer();
        // A backing store is sent once, and again when its buffer changes (as a resize makes new ones).
        auto sent = m_sent_buffers.get(bitmap_id);
        if (!sent.has_value() || *sent != buffer.data<void>()) {
            Writer store { ToZgx::Store };
            store.u64_(m_id)
                .i32_(bitmap_id)
                .u32_(bitmap->width())
                .u32_(bitmap->height())
                .u32_(bitmap->pitch())
                .u8_(static_cast<u8>(bitmap->format()))
                .u8_(bitmap->alpha_type() == Gfx::AlphaType::Premultiplied ? 1 : 0)
                .u64_(buffer.size());
            s_connection->send(store, buffer.fd());
            m_sent_buffers.set(bitmap_id, buffer.data<void>());
        }
#endif
#if defined(AK_OS_MACOS)
        // Here the Compositor's stores are IOSurfaces, which reach another process as Mach ports. Until the app imports
        // them, each store has a shared buffer of the host's that what changed of each frame is copied into.
        auto bitmap = frame->shared_image_buffer->bitmap_if_present();
        if (!bitmap)
            return;
        auto size = static_cast<size_t>(bitmap->pitch()) * bitmap->height();
        auto copy = m_copies.get(bitmap_id);
        bool fresh = !copy.has_value() || copy->size() != size;
        if (fresh) {
            auto buffer = Core::AnonymousBuffer::create_with_size(size);
            if (buffer.is_error())
                return;
            m_copies.set(bitmap_id, buffer.release_value());
            copy = m_copies.get(bitmap_id);
            Writer store { ToZgx::Store };
            store.u64_(m_id)
                .i32_(bitmap_id)
                .u32_(bitmap->width())
                .u32_(bitmap->height())
                .u32_(bitmap->pitch())
                .u8_(static_cast<u8>(bitmap->format()))
                .u8_(bitmap->alpha_type() == Gfx::AlphaType::Premultiplied ? 1 : 0)
                .u64_(size);
            s_connection->send(store, copy->fd());
        }
        // The rows this store missed: what changed since it last showed a frame, which is everything a newer frame in
        // another store changed too. Copying the whole damage of the frames since keeps it simple: the union of what
        // the stores' frames changed since this one was filled.
        auto rect = fresh ? bitmap->rect() : m_pending_damage.get(bitmap_id).value_or(bitmap->rect()).united(damage).intersected(bitmap->rect());
        auto const* source = reinterpret_cast<u8 const*>(bitmap->scanline_u8(0));
        auto* target = copy->data<u8>();
        for (int y = rect.top(); y < rect.bottom(); ++y)
            __builtin_memcpy(target + y * bitmap->pitch() + rect.left() * 4, source + y * bitmap->pitch() + rect.left() * 4, rect.width() * 4);
        // Every other store now lacks this frame's change.
        for (auto& [id, pending] : m_pending_damage) {
            if (id != bitmap_id)
                pending = pending.united(damage);
        }
        for (auto& [id, _] : m_copies) {
            if (id != bitmap_id && !m_pending_damage.contains(id))
                m_pending_damage.set(id, damage);
        }
        m_pending_damage.remove(bitmap_id);
#endif
        m_last_sent_bitmap = bitmap_id;
        Writer present { ToZgx::Present };
        present.u64_(m_id)
            .i32_(bitmap_id)
            .i32_(damage.x())
            .i32_(damage.y())
            .i32_(damage.width())
            .i32_(damage.height())
            .u32_(frame->size.width())
            .u32_(frame->size.height())
            .f64_(frame->device_pixel_ratio);
        s_connection->send(present);
    }

    // A paste reads the app's clipboard (the Application's, below), not the copy LibWebView keeps.
    virtual void read_clipboard_item(Function<void(Web::Clipboard::SystemClipboardItem)> on_read) const override
    {
        WebView::Application::the().read_clipboard_item(move(on_read));
    }

    // Held until the app is done with the frames before (`done`).
    virtual void release_backing_store(i32 bitmap_id) override
    {
        if (bitmap_id < 0)
            return;
        m_held_releases.append(bitmap_id);
    }

    u64 m_id { 0 };
    HashMap<i32, void const*> m_sent_buffers;
#if defined(AK_OS_MACOS)
    HashMap<i32, Core::AnonymousBuffer> m_copies;
    HashMap<i32, Gfx::IntRect> m_pending_damage;
#endif
    Vector<i32> m_held_releases;
    i32 m_last_sent_bitmap { -1 };
    Optional<u64> m_snapshot_request;
};

class Application final : public WebView::Application {
    WEB_VIEW_APPLICATION(Application)

public:
    Application()
        : WebView::Application(OptionalNone {})
    {
    }

    int socket_fd { -1 };

    virtual void create_platform_arguments(Core::ArgsParser& args_parser) override
    {
        args_parser.add_option(socket_fd, "The socket to the zgx app", "zgx-socket", 0, "fd");
    }

    virtual bool should_coordinate_browser_process() const override { return false; }

    // The clipboard is the app's: what pages copy goes to it, and pastes read it (`ToZgx::ClipboardWrite`, `ClipboardRead`,
    // answered by `ToHost::ClipboardData`).
    virtual void insert_clipboard_item(Web::Clipboard::SystemClipboardItem item) override
    {
        ByteString text, html;
        for (auto const& representation : item.system_clipboard_representations) {
            auto const* data = representation.data.get_pointer<ByteString>();
            if (!data)
                continue;
            if (representation.name == "text/plain"sv)
                text = *data;
            else if (representation.name == "text/html"sv)
                html = *data;
        }
        WebView::Application::insert_clipboard_item(move(item));
        Writer writer { ToZgx::ClipboardWrite };
        writer.u64_(0).str(text).str(html);
        if (s_connection)
            s_connection->send(writer);
    }
    virtual void set_clipboard_text(String text, ClipboardType type) override
    {
        WebView::Application::set_clipboard_text(text, type);
        Writer writer { ToZgx::ClipboardWrite };
        writer.u64_(0).str(text.bytes_as_string_view()).str(""sv);
        if (s_connection)
            s_connection->send(writer);
    }
    virtual void read_clipboard_item(Function<void(Web::Clipboard::SystemClipboardItem)> on_read) const override
    {
        auto request = ++m_next_clipboard_read;
        m_clipboard_reads.set(request, move(on_read));
        Writer writer { ToZgx::ClipboardRead };
        writer.u64_(0).u64_(request);
        if (s_connection)
            s_connection->send(writer);
    }
    virtual void read_clipboard_text(ClipboardType, Function<void(Utf16String)> on_read) const override
    {
        read_clipboard_item([on_read = move(on_read)](Web::Clipboard::SystemClipboardItem item) {
            for (auto const& representation : item.system_clipboard_representations) {
                if (representation.name == "text/plain"sv) {
                    if (auto const* data = representation.data.get_pointer<ByteString>()) {
                        on_read(Utf16String::from_utf8(*data));
                        return;
                    }
                }
            }
            on_read({});
        });
    }
    void answer_clipboard(u64 request, ByteString text, ByteString html)
    {
        auto on_read = m_clipboard_reads.take(request);
        if (!on_read.has_value())
            return;
        Web::Clipboard::SystemClipboardItem item;
        if (!text.is_empty())
            item.system_clipboard_representations.append({ .name = "text/plain"_string, .data = move(text) });
        if (!html.is_empty())
            item.system_clipboard_representations.append({ .name = "text/html"_string, .data = move(html) });
        (*on_read)(move(item));
    }

    mutable u64 m_next_clipboard_read { 0 };
    mutable HashMap<u64, Function<void(Web::Clipboard::SystemClipboardItem)>> m_clipboard_reads;
    virtual bool should_use_os_geolocation() const override { return false; }
};

static Web::UIEvents::KeyModifier modifiers_from(u8 bits)
{
    return static_cast<Web::UIEvents::KeyModifier>(bits & Web::UIEvents::Mod_Mask);
}

class Host {
public:
    Host(Connection& connection, Core::AnonymousBuffer theme)
        : m_connection(connection)
        , m_theme(move(theme))
    {
        m_connection.on_message = [this](ToHost kind, Reader& reader) { handle(kind, reader); };
        m_connection.on_closed = [] { Core::EventLoop::current().quit(0); };
        Writer ready { ToZgx::Ready };
        m_connection.send(ready);
    }

private:
    View* view(u64 id)
    {
        auto it = m_views.find(id);
        return it == m_views.end() ? nullptr : it->value.ptr();
    }

    void handle(ToHost kind, Reader& reader)
    {
        auto id = reader.u64_();
        if (kind == ToHost::ClipboardData) {
            auto request = reader.u64_();
            auto text = reader.str();
            auto html = reader.str();
            if (!reader.failed())
                static_cast<Application&>(WebView::Application::the()).answer_clipboard(request, move(text), move(html));
            return;
        }
        if (kind == ToHost::CreateView) {
            auto width = reader.u32_();
            auto height = reader.u32_();
            auto dpr = reader.f64_();
            if (reader.failed() || m_views.contains(id))
                return;
            m_views.set(id, View::create(id, m_theme, { width, height }, dpr));
            return;
        }
        auto* target = view(id);
        if (!target)
            return;
        switch (kind) {
        case ToHost::CloseView:
            m_views.remove(id);
            break;
        case ToHost::LoadUrl: {
            auto text = reader.str();
            if (auto url = URL::Parser::basic_parse(text); url.has_value())
                target->load(*url);
            break;
        }
        case ToHost::LoadHtml:
            target->load_html(reader.str());
            break;
        case ToHost::Reload:
            target->reload();
            break;
        case ToHost::Stop:
            target->stop_loading();
            break;
        case ToHost::Traverse:
            target->traverse_the_history_by_delta(reader.i32_());
            break;
        case ToHost::Resize: {
            auto width = reader.u32_();
            auto height = reader.u32_();
            auto dpr = reader.f64_();
            if (!reader.failed())
                target->resize({ width, height }, dpr);
            break;
        }
        case ToHost::Mouse: {
            Web::MouseEvent event {};
            event.type = static_cast<Web::MouseEvent::Type>(reader.u8_());
            auto x = reader.f64_();
            auto y = reader.f64_();
            event.position = { x, y };
            event.screen_position = event.position;
            event.button = static_cast<Web::UIEvents::MouseButton>(reader.u8_());
            event.buttons = static_cast<Web::UIEvents::MouseButton>(reader.u8_());
            event.modifiers = modifiers_from(reader.u8_());
            event.click_count = reader.u8_();
            if (!reader.failed())
                target->enqueue_input_event(move(event));
            break;
        }
        case ToHost::Wheel: {
            Web::MouseEvent event {};
            event.type = Web::MouseEvent::Type::MouseWheel;
            auto x = reader.f64_();
            auto y = reader.f64_();
            event.position = { x, y };
            event.screen_position = event.position;
            event.wheel_delta_x = reader.f64_();
            event.wheel_delta_y = reader.f64_();
            event.wheel_delta_precision = reader.u8_() ? Web::WheelDeltaPrecision::Precise : Web::WheelDeltaPrecision::Discrete;
            event.modifiers = modifiers_from(reader.u8_());
            event.wheel_tick_120ths_x = Web::wheel_tick_120ths_from_pixel_delta(event.wheel_delta_x, event.wheel_delta_precision);
            event.wheel_tick_120ths_y = Web::wheel_tick_120ths_from_pixel_delta(event.wheel_delta_y, event.wheel_delta_precision);
            if (!reader.failed())
                target->enqueue_input_event(move(event));
            break;
        }
        case ToHost::Key: {
            Web::KeyEvent event {};
            event.type = static_cast<Web::KeyEvent::Type>(reader.u8_());
            event.key = static_cast<Web::UIEvents::KeyCode>(reader.u32_());
            event.modifiers = modifiers_from(reader.u8_());
            event.code_point = reader.u32_();
            event.repeat = reader.u8_() != 0;
            event.should_insert_text = event.type == Web::KeyEvent::Type::KeyDown && event.code_point != 0;
            if (!reader.failed())
                target->enqueue_input_event(move(event));
            break;
        }
        case ToHost::Focus:
            target->set_has_system_focus(reader.u8_() != 0);
            break;
        case ToHost::Done:
            target->done(reader.i32_());
            break;
        case ToHost::Zoom:
            target->set_zoom(reader.f64_());
            break;
        case ToHost::RunScript:
            target->run_javascript(MUST(String::from_byte_string(reader.str())));
            break;
        case ToHost::ImeMark:
            target->set_marked_text_from_input_method(Utf16String::from_utf8(MUST(String::from_byte_string(reader.str()))));
            break;
        case ToHost::ImeCommit:
            target->commit_text_from_input_method(Utf16String::from_utf8(MUST(String::from_byte_string(reader.str()))));
            break;
        case ToHost::ImeUnmark:
            target->unmark_text_from_input_method();
            break;
        case ToHost::ReloadIgnoringCache:
            target->reload(WebView::BypassCache::Yes);
            break;
        case ToHost::Edit:
            target->edit(reader.u8_());
            break;
        case ToHost::Snapshot:
            target->snapshot(reader.u64_());
            break;
        case ToHost::Visible:
            target->set_system_visibility_state(reader.u8_() ? Web::HTML::VisibilityState::Visible : Web::HTML::VisibilityState::Hidden);
            break;
        default:
            break;
        }
    }

    Connection& m_connection;
    Core::AnonymousBuffer m_theme;
    HashMap<u64, NonnullOwnPtr<View>> m_views;
};

}

ErrorOr<int> ladybird_main(Main::Arguments arguments)
{
    auto app = TRY(ZgxHost::Application::create(arguments));
    if (app->socket_fd < 0) {
        warnln("zgx-ladybird-host runs under a zgx app, which passes --zgx-socket");
        return 1;
    }
    auto theme_path = LexicalPath::join(WebView::s_ladybird_resource_root, "themes"sv, "Default.ini"sv);
    auto theme = TRY(Gfx::load_system_theme(theme_path.string()));

    ZgxHost::Connection connection { app->socket_fd };
    ZgxHost::s_connection = &connection;
    ZgxHost::Host host { connection, move(theme) };
    return app->execute();
}
