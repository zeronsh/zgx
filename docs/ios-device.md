# A zgx app on a real iPhone

Plug the phone in with a cable, unlock it, and tap Trust if it asks. Then, from a checkout on the branch:

```sh
python3 scripts/ios/device.py check
```

It lists the phones this Mac sees, its signing identities, and whether a provisioning profile covers the app.

## One setup step, once

An app on a phone needs a provisioning profile. If `check` says none covers `dev.zgx.ios.app`:

```sh
python3 scripts/ios/device.py setup
```

This builds an empty app with Xcode's automatic signing, which makes the profile. **macOS may ask for your Mac password or for access to the signing key. That dialog is yours to answer**: choose Always Allow and later runs won't ask. Every signing step has a timeout, so a waiting dialog ends the step with a message rather than hanging.

Without xcodegen: in Xcode, make any iOS app with the bundle identifier `dev.zgx.ios.app` and your team, under Signing & Capabilities, and run it on the phone once.

The first time a phone runs an app from a new developer it refuses until you allow it, on the phone: Settings, General, VPN & Device Management, your developer name, Trust. iOS 16 and later also ask you to turn on Developer Mode: Settings, Privacy & Security, Developer Mode, which restarts the phone.

## Run something

```sh
python3 scripts/ios/device.py run --example glass
python3 scripts/ios/device.py run -p zgx-run --example ios_app --resource crates/zgx-script/examples/ios_settings
python3 scripts/ios/device.py run -p zgx-run --example ios_app --resource crates/zgx-script/examples/ios_showcase --console
```

With several phones paired, add `--device "name"`. Add `--env NAME=value` for the app's environment. The app is always `zgx` on the home screen; each run replaces the last.

## Measure glass against no glass

```sh
python3 scripts/ios/device.py measure --seconds 30
```

Keep the phone unlocked with its screen on and Auto-Lock off. It installs and runs each scene (idle and scrolling; none, one and four panes; text over the glass; drawn and hosted switches) and prints a table, keeping every window in `target/ios-device/`:

| Column | Means |
| --- | --- |
| `cpu_pct` | the app's CPU, % of one core |
| `gpu_app_pct`, `gpu_ms_per_buffer` | GPU time of zgx's own command buffers |
| `gap_p50_ms` … `gap_max_ms` | how late the main thread's display callback ran: 8.3 ms is a steady 120 Hz |
| `memory_mb` | the footprint the system charges the app |
| `thermal` | 0 nominal, 1 fair, 2 serious, 3 critical |

**What this cannot see is the compositor.** Glass is drawn by the system's render server, so its GPU cost is not in `gpu_app_pct`. For that, in Instruments: choose the phone, the Metal System Trace or Game Performance template, the `zgx` process, and record 20 seconds of each scene; read GPU utilization and the render server's track. The number that decides the risk is the *difference* between `scrolling, no glass` and `scrolling, 4 glass`. Leave a scene running for 5 to 10 minutes (`--seconds 300`) to see whether `thermal` climbs and `battery` falls.

Also worth checking on a phone, because a simulator can't say:

- Fonts: text shows in San Francisco (it reads `/System/Library/Fonts`).
- Feel: flick, overscroll, press and hold on glass, the tab bar's lens, a real tap on the status bar.
- VoiceOver, Switch Control, Voice Control and Full Keyboard Access, in Settings, Accessibility.
