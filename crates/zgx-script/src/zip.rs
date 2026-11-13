//! `ui.zip(file)`: a `.zip` written into a file the user picked
//! (`ui.save_file`), one entry at a time, so a script never holds the whole
//! archive (a plugin's memory is capped):
//!
//! ```luau
//! local z = ui.zip(file)
//! z:add("01-home.png", { base64 = png })   -- or a string, as text
//! z:add("index.html", html)
//! z:close():after(function() end)          -- after every add, in order
//! ```
//!
//! Entries are stored as given (no compression: pictures are compressed
//! already), names in UTF-8, written by one thread in the order they were
//! added. The archive is built beside the file and renamed over it on
//! `close`, so a zip left unclosed (or failing) leaves the file as it was.
use crate::{
    calls::{PromiseUd, pending, settle_with},
    value::Value,
};
use base64::Engine as _;
use mlua::{Lua, UserData, UserDataMethods};
use std::{
    cell::RefCell,
    collections::HashSet,
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::mpsc,
};

/// The most entries, and bytes in all, a zip without Zip64 holds.
const MAX_ENTRIES: usize = 65_534;
const MAX_BYTES: u64 = u32::MAX as u64 - (64 << 20);

enum Job {
    Add(String, Vec<u8>, zgx::task::LatestSender<Result<(), String>>),
    Close(zgx::task::LatestSender<Result<(), String>>),
}

struct Entry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

/// The writer thread's: the archive so far.
struct Archive {
    out: BufWriter<File>,
    temp: PathBuf,
    path: PathBuf,
    entries: Vec<Entry>,
    at: u64,
}

// (MS-DOS date of 1980-01-01, the format's earliest: entries carry no time.)
const DOS_DATE: u16 = (1 << 5) | 1;

impl Archive {
    fn add(&mut self, name: String, bytes: &[u8]) -> Result<(), String> {
        if self.entries.len() >= MAX_ENTRIES {
            return Err(format!("{MAX_ENTRIES} entries at most"));
        }
        if self.at + bytes.len() as u64 + name.len() as u64 + 30 > MAX_BYTES {
            return Err("a zip holds 4 GB at most".into());
        }
        let crc = crc32fast::hash(bytes);
        let mut head = Vec::with_capacity(30 + name.len());
        head.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        head.extend_from_slice(&20u16.to_le_bytes()); // version needed
        head.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        head.extend_from_slice(&0u16.to_le_bytes()); // stored
        head.extend_from_slice(&0u16.to_le_bytes()); // time
        head.extend_from_slice(&DOS_DATE.to_le_bytes());
        head.extend_from_slice(&crc.to_le_bytes());
        head.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        head.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        head.extend_from_slice(&(name.len() as u16).to_le_bytes());
        head.extend_from_slice(&0u16.to_le_bytes()); // extra
        head.extend_from_slice(name.as_bytes());
        self.out.write_all(&head).and_then(|()| self.out.write_all(bytes)).map_err(|e| e.to_string())?;
        self.entries.push(Entry { name, crc, size: bytes.len() as u32, offset: self.at as u32 });
        self.at += (head.len() + bytes.len()) as u64;
        Ok(())
    }

    fn close(mut self) -> Result<(), String> {
        let start = self.at;
        let mut dir = Vec::new();
        for e in &self.entries {
            dir.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            dir.extend_from_slice(&20u16.to_le_bytes()); // made by
            dir.extend_from_slice(&20u16.to_le_bytes()); // needed
            dir.extend_from_slice(&0x0800u16.to_le_bytes());
            dir.extend_from_slice(&0u16.to_le_bytes());
            dir.extend_from_slice(&0u16.to_le_bytes());
            dir.extend_from_slice(&DOS_DATE.to_le_bytes());
            dir.extend_from_slice(&e.crc.to_le_bytes());
            dir.extend_from_slice(&e.size.to_le_bytes());
            dir.extend_from_slice(&e.size.to_le_bytes());
            dir.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            dir.extend_from_slice(&[0; 8]); // extra, comment, disk, attributes (internal)
            dir.extend_from_slice(&0u32.to_le_bytes()); // attributes (external)
            dir.extend_from_slice(&e.offset.to_le_bytes());
            dir.extend_from_slice(e.name.as_bytes());
        }
        let n = self.entries.len() as u16;
        dir.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        dir.extend_from_slice(&[0; 4]); // disks
        dir.extend_from_slice(&n.to_le_bytes());
        dir.extend_from_slice(&n.to_le_bytes());
        dir.extend_from_slice(&((dir.len() - 12) as u32).to_le_bytes());
        dir.extend_from_slice(&(start as u32).to_le_bytes());
        dir.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.out.write_all(&dir).and_then(|()| self.out.flush()).map_err(|e| e.to_string())?;
        drop(self.out);
        std::fs::rename(&self.temp, &self.path).map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

/// An entry's name: a relative path with `/`, no `..`, not empty.
fn entry_name(name: &str) -> Result<String, String> {
    let name = name.replace('\\', "/");
    if name.is_empty()
        || name.len() > u16::MAX as usize
        || name.starts_with('/')
        || name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || name.contains(':')
    {
        return Err(format!("\"{name}\" isn't a name inside a zip (a relative path, with no `..`)"));
    }
    Ok(name)
}

pub(crate) struct ZipUd {
    jobs: RefCell<Option<mpsc::Sender<Job>>>,
    names: RefCell<HashSet<String>>,
}

/// Starts the zip at `path` (a picked file): its writer thread.
pub(crate) fn open(path: PathBuf) -> Result<ZipUd, String> {
    let temp = path.with_extension("zgx-zip");
    let file = File::create(&temp).map_err(|e| format!("{}: {e}", temp.display()))?;
    let (jobs, heard) = mpsc::channel::<Job>();
    std::thread::spawn(move || {
        let mut archive = Some(Archive { out: BufWriter::new(file), temp: temp.clone(), path, entries: Vec::new(), at: 0 });
        let mut failed: Option<String> = None;
        for job in heard {
            match job {
                Job::Add(name, bytes, tell) => {
                    let done = match (&failed, archive.as_mut()) {
                        (Some(why), _) => Err(why.clone()),
                        (None, Some(a)) => a.add(name, &bytes),
                        (None, None) => Err("this zip is closed".into()),
                    };
                    if let Err(why) = &done {
                        failed.get_or_insert_with(|| why.clone());
                    }
                    tell.send(done);
                }
                Job::Close(tell) => {
                    let done = match (failed.take(), archive.take()) {
                        (Some(why), _) => Err(why),
                        (None, Some(a)) => a.close(),
                        (None, None) => Err("this zip is closed".into()),
                    };
                    tell.send(done);
                }
            }
        }
        // Never closed (or failed): nothing replaces the file.
        drop(archive);
        let _ = std::fs::remove_file(&temp);
    });
    Ok(ZipUd { jobs: RefCell::new(Some(jobs)), names: RefCell::new(HashSet::new()) })
}

impl ZipUd {
    fn send(&self, lua: &Lua, what: &str, job: impl FnOnce(zgx::task::LatestSender<Result<(), String>>) -> Job) -> mlua::Result<PromiseUd> {
        let (tell, mut heard) = zgx::task::latest::<Result<(), String>>();
        let promise = pending(lua, what.into())?;
        let sent = match self.jobs.borrow().as_ref() {
            Some(jobs) => jobs.send(job(tell)).is_ok(),
            None => false,
        };
        let closed = !sent;
        settle_with(lua, &promise, crate::errors::Kind::Io, async move {
            if closed {
                return Err("ui.zip: this zip is closed".into());
            }
            match heard.recv().await {
                Some(Ok(())) => Ok(Value::Nil),
                Some(Err(error)) => Err(format!("ui.zip: {error}")),
                None => Err("ui.zip: the zip's thread stopped".into()),
            }
        })?;
        Ok(PromiseUd(promise))
    }
}

impl UserData for ZipUd {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("add", |lua, zip, (name, data): (String, mlua::Value)| {
            let name = entry_name(&name).map_err(|e| mlua::Error::runtime(format!("zip:add: {e}")))?;
            if !zip.names.borrow_mut().insert(name.clone()) {
                return Err(mlua::Error::runtime(format!("zip:add: \"{name}\" is in this zip already")));
            }
            let bytes = match data {
                mlua::Value::String(text) => text.as_bytes().to_vec(),
                mlua::Value::Table(t) => {
                    let encoded = t.get::<String>("base64").map_err(|_| {
                        mlua::Error::runtime("zip:add: a table gives the bytes as `base64`")
                    })?;
                    base64::engine::general_purpose::STANDARD
                        .decode(encoded.trim())
                        .map_err(|e| mlua::Error::runtime(format!("zip:add: not base64 ({e})")))?
                }
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "zip:add writes a string or {{ base64 = … }}, not {}",
                        other.type_name()
                    )));
                }
            };
            zip.send(lua, "zip:add", |tell| Job::Add(name, bytes, tell))
        });
        methods.add_method("close", |lua, zip, ()| {
            let promise = zip.send(lua, "zip:close", Job::Close);
            // Nothing after it: the thread ends once the close is done.
            zip.jobs.borrow_mut().take();
            promise
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(dir: &std::path::Path) -> Archive {
        let path = dir.join("out.zip");
        let temp = path.with_extension("zgx-zip");
        Archive { out: BufWriter::new(File::create(&temp).unwrap()), temp, path, entries: Vec::new(), at: 0 }
    }

    /// What a zip reader finds: each entry's name and bytes, from the
    /// central directory (as unzip reads it).
    fn read(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let end = bytes.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50);
        let (n, mut at) = (u16_at(end + 10), u32_at(end + 16));
        let mut out = Vec::new();
        for _ in 0..n {
            assert_eq!(u32_at(at), 0x0201_4b50);
            let (crc, size, name_len, offset) = (u32_at(at + 16), u32_at(at + 20), u16_at(at + 28), u32_at(at + 42));
            let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec()).unwrap();
            assert_eq!(u32_at(offset), 0x0403_4b50);
            let data_at = offset + 30 + u16_at(offset + 26) + u16_at(offset + 28);
            let data = bytes[data_at..data_at + size].to_vec();
            assert_eq!(crc32fast::hash(&data) as usize, crc, "{name}");
            out.push((name, data));
            at += 46 + name_len;
        }
        out
    }

    #[test]
    fn entries_come_out_as_they_went_in() {
        let dir = std::env::temp_dir().join(format!("zgx-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = archive(&dir);
        let png = [0x89u8, b'P', b'N', b'G', 0, 255];
        a.add("01-trang-chủ.png".into(), &png).unwrap();
        a.add("code/index.html".into(), b"<p>hi</p>").unwrap();
        a.add("empty.txt".into(), b"").unwrap();
        a.close().unwrap();
        let got = read(&std::fs::read(dir.join("out.zip")).unwrap());
        assert_eq!(
            got,
            vec![
                ("01-trang-chủ.png".to_string(), png.to_vec()),
                ("code/index.html".to_string(), b"<p>hi</p>".to_vec()),
                ("empty.txt".to_string(), Vec::new()),
            ]
        );
        assert!(!dir.join("out.zgx-zip").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_must_stay_inside() {
        for bad in ["", "/abs.png", "../up.png", "a/../b", "a//b", "C:/x", "./x"] {
            assert!(entry_name(bad).is_err(), "{bad}");
        }
        assert_eq!(entry_name("a\\b.png").unwrap(), "a/b.png");
    }
}
