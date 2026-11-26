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
        on_cursor_change = [this](Gfx::Cursor const& cursor) {
            u8 standard = 0;
            if (auto const* named = cursor.get_pointer<Gfx::StandardCursor>())
                standard = static_cast<u8>(*named);
            Writer writer { ToZgx::Cursor };
            writer.u64_(m_id).u8_(standard);
            s_connection->send(writer);
        };
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
