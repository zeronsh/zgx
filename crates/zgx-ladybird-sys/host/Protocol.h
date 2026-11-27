/*
 * The messages between zgx and its Ladybird host, over one stream socket. The Rust side
 * (zgx-desktop's native_host/ladybird/protocol.rs) is the reference: keep the two in step.
 *
 * A message is a little-endian u32 length (of what follows), a u8 kind and its fields: integers and floats as
 * themselves, strings as a u32 byte count and UTF-8. A message that carries file descriptors (Store) sends them with
 * its first byte (SCM_RIGHTS); the reader queues descriptors as they arrive and each such message takes its own.
 */

#pragma once

#include <AK/ByteBuffer.h>
#include <AK/ByteString.h>
#include <AK/Optional.h>
#include <AK/String.h>
#include <AK/Types.h>
#include <AK/Vector.h>

namespace ZgxHost {

enum class ToHost : u8 {
    CreateView = 1,
    CloseView = 2,
    LoadUrl = 3,
    LoadHtml = 4,
    Reload = 5,
    Stop = 6,
    Traverse = 7,
    Resize = 8,
    Mouse = 9,
    Wheel = 10,
    Key = 11,
    Focus = 12,
    Done = 13,
    Zoom = 14,
    RunScript = 15,
    ImeMark = 16,
    ImeCommit = 17,
    ImeUnmark = 18,
    Visible = 19,
    Snapshot = 20,
    ReloadIgnoringCache = 21,
    Edit = 22,
    ClipboardData = 23,
};

enum class ToZgx : u8 {
    Ready = 101,
    Store = 102,
    Present = 103,
    Title = 104,
    Url = 105,
    Loading = 106,
    LoadFinished = 107,
    History = 108,
    Cursor = 109,
    NewWindow = 110,
    Crashed = 111,
    Editing = 112,
    LinkHovered = 113,
    ThemeColor = 114,
    Snapshot = 115,
    ClipboardWrite = 116,
    ClipboardRead = 117,
    Favicon = 118,
};

class Writer {
public:
    explicit Writer(ToZgx kind)
    {
        m_bytes.append(0);
        m_bytes.append(0);
        m_bytes.append(0);
        m_bytes.append(0);
        m_bytes.append(static_cast<u8>(kind));
    }

    template<typename T>
    Writer& raw(T value)
    {
        u8 bytes[sizeof(T)];
        __builtin_memcpy(bytes, &value, sizeof(T));
        for (auto byte : bytes)
            m_bytes.append(byte);
        return *this;
    }
    Writer& u8_(u8 value) { return raw(value); }
    Writer& u32_(u32 value) { return raw(value); }
    Writer& i32_(i32 value) { return raw(value); }
    Writer& u64_(u64 value) { return raw(value); }
    Writer& f64_(double value) { return raw(value); }
    Writer& bytes(ReadonlyBytes value)
    {
        raw(static_cast<u32>(value.size()));
        for (auto byte : value)
            m_bytes.append(byte);
        return *this;
    }
    Writer& str(StringView value)
    {
        raw(static_cast<u32>(value.length()));
        for (auto c : value)
            m_bytes.append(static_cast<u8>(c));
        return *this;
    }

    Vector<u8> const& finish()
    {
        u32 length = m_bytes.size() - 4;
        __builtin_memcpy(m_bytes.data(), &length, 4);
        return m_bytes;
    }

private:
    Vector<u8> m_bytes;
};

class Reader {
public:
    Reader(ReadonlyBytes bytes)
        : m_bytes(bytes)
    {
    }

    template<typename T>
    T raw()
    {
        T value {};
        if (m_offset + sizeof(T) > m_bytes.size()) {
            m_failed = true;
            return value;
        }
        __builtin_memcpy(&value, m_bytes.data() + m_offset, sizeof(T));
        m_offset += sizeof(T);
        return value;
    }
    u8 u8_() { return raw<u8>(); }
    u32 u32_() { return raw<u32>(); }
    i32 i32_() { return raw<i32>(); }
    u64 u64_() { return raw<u64>(); }
    double f64_() { return raw<double>(); }
    ByteString str()
    {
        auto length = u32_();
        if (m_failed || m_offset + length > m_bytes.size()) {
            m_failed = true;
            return {};
        }
        ByteString value { reinterpret_cast<char const*>(m_bytes.data() + m_offset), length };
        m_offset += length;
        return value;
    }
    bool failed() const { return m_failed; }

private:
    ReadonlyBytes m_bytes;
    size_t m_offset { 0 };
    bool m_failed { false };
};

}
