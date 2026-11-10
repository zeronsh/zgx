#!/usr/bin/env python3
"""Build an isolated real Zeron fixture host against the chosen zgx checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
from gate import touch

ROOT = Path(__file__).resolve().parents[2]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source', type=Path, required=True, help='isolated zeron/laurent-chips checkout')
    p.add_argument('--zgx', type=Path, default=ROOT)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--target', type=Path, default=ROOT / 'target/zeron-build-cache')
    a = p.parse_args()
    source, engine, output = a.source.resolve(), a.zgx.resolve(), a.output.resolve()
    app = source / 'apps/zgx' if (source/'apps/zgx').is_dir() else source
    if not (app/'ui/shell_fixture.luau').is_file():
        p.error('source must contain the actual shell_fixture.luau')
    main_rs = app/'src/main.rs'
    text = main_rs.read_text()
    if '// Scroll benchmark fixture geometry.' not in text:
        needle = '        Screen::Gallery | Screen::Fixture(_) => gallery_window(),'
        if needle not in text:
            raise RuntimeError('unrecognized fixture window constructor')
        text = text.replace(needle, '''        Screen::Gallery => gallery_window(),
        // Scroll benchmark fixture geometry. The existing capture override
        // disables frame persistence and sets exact physical bounds.
        Screen::Fixture(_) => {
            let mut window = placed(gallery_window());
            window.min_size = Some((1., 1.));
            window
        },''')
        needle = '    // The network, for `ui.fetch` and remote images:'
        if needle not in text:
            raise RuntimeError('unrecognized script host constructor')
        at = text.index(needle)
        text = text[:at]+'''    // Scroll benchmark reference: the real shell, with a tiny marker
    // advanced on the display clock rather than a jittering timer.
    if std::env::var("ZGX_SCROLL_REFERENCE").is_ok_and(|v| v == "1") {
        host.shared("scroll.reference", true);
        let beat = host.shared("scroll.reference.beat", false);
        let source = cx.frames.clone();
        cx.tasks.spawn(async move {
            let mut value = false;
            loop {
                source.next().await;
                value = !value;
                beat.set(Value::Bool(value));
            }
        });
    }
'''+text[at:]
        main_rs.write_text(text)
    text = main_rs.read_text()
    if '// Scroll fixture data is supplied by shell_fixture.' not in text:
        text = text.replace('        chat::install(cx, &host);', '''        // Scroll fixture data is supplied by shell_fixture. A disconnected
        // engine must not replace it with nil when its connection times out.
        if entry != "scroll_fixture.luau" {
            chat::install(cx, &host);
        }''')
        main_rs.write_text(text)
    manifest = app/'Cargo.toml'
    text = manifest.read_text()
    if '[profile.dist]' not in text:
        manifest.write_text(text+'\n[profile.dist]\ninherits = "release"\nlto = "fat"\ncodegen-units = 1\n')
    # Cargo applies these overrides to the app and its path dependencies.
    # No installed app or existing user data is opened or changed.
    patches = []
    for directory in list((engine/'crates').iterdir())+list((engine/'vendor').iterdir()):
        cargo = directory/'Cargo.toml'
        if not cargo.is_file():
            continue
        match = re.search(r'(?m)^name\s*=\s*"([^"]+)"',cargo.read_text())
        if match:
            patches.append((match.group(1),directory))
    # Rewrite direct and crates.io-patch git references too: Cargo can otherwise
    # retain a second git copy of a links crate beside its local path copy.
    by_name = dict(patches)
    text = manifest.read_text()
    pattern = r'(?m)^(\w[\w-]*) = \{ git = "https://github.com/zeronsh/zgx", rev = "[^"]+"(.*?)\}$'
    def local_dependency(match):
        name, rest = match.groups()
        if name not in by_name:
            raise RuntimeError(f'no local zgx dependency {name}')
        return f'{name} = {{ path = {json.dumps(str(by_name[name]))}{rest}}}'
    text = re.sub(pattern, local_dependency, text)
    for name, directory in by_name.items():
        pattern = rf'(?m)^{re.escape(name)} = \{{ path = "[^"]+"(.*?)\}}$'
        text = re.sub(pattern, lambda m: f'{name} = {{ path = {json.dumps(str(directory))}{m[1]}}}', text)
    manifest.write_text(text)
    config = app/'.cargo/config.toml'
    config.parent.mkdir(exist_ok=True)
    lines = ['[net]','git-fetch-with-cli = true','','[patch."https://github.com/zeronsh/zgx"]']
    lines += [f'{name} = {{ path = {json.dumps(str(path))} }}' for name,path in sorted(patches)]
    config.write_text('\n'.join(lines)+'\n')
    output.mkdir(parents=True,exist_ok=True)
    env = dict(os.environ,CARGO_TARGET_DIR=str(a.target.resolve()),ZIG='/opt/homebrew/opt/zig@0.16/bin/zig')
    if not Path(env['ZIG']).is_file():
        env.pop('ZIG')
    # A target cache copied from another checkout can otherwise look newer
    # than this checkout's changed sources. Reuse the normal perf builder's
    # source freshness rule before every measured native build.
    touch(engine)
    touch(source)
    subprocess.run(['cargo','build','--profile','dist','--manifest-path',str(manifest),
                    '--bin','zeron-zgx'],cwd=app,env=env,check=True)
    binary = output/'zeron-zgx'
    shutil.copy2(a.target.resolve()/'dist/zeron-zgx',binary)
    shutil.copy2(app/'Cargo.lock',output/'Cargo.lock')
    git = lambda root,*args: subprocess.check_output(['git',*args],cwd=root,text=True)
    proof = dict(zeron_commit=git(source,'rev-parse','HEAD').strip(),
                 zgx_commit=git(engine,'rev-parse','HEAD').strip(),
                 zeron_diff=git(source,'diff'), zgx_diff=git(engine,'diff','HEAD'),
                 cargo_config=config.read_text(), beta=False,
                 binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                 lock_sha256=hashlib.sha256((app/'Cargo.lock').read_bytes()).hexdigest())
    (output/'build.json').write_text(json.dumps(proof,indent=2)+'\n')
    print(binary)


if __name__ == '__main__':
    main()
