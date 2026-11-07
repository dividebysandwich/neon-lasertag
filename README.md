# Neon Lasertag

Red vs Blue lasertag in procedurally generated neon arenas, for VR headsets (OpenXR) and the
flat screen. Built with Bevy.

<img width="1916" height="1051" alt="image" src="https://github.com/user-attachments/assets/1a696844-08a1-4cbf-84e2-67545a2d226c" />

## Getting started

Download a build for your platform from the releases page, or run from source:

```
cargo run --release                         # VR if a headset is available, else the flat screen
cargo run --release -- --flat               # flat screen
cargo run --release -- --seed 42 --bots 7   # straight into a match against bots
```

The start menu offers **Botmatch**, **Multiplayer** (Internet server list or LAN, join or host),
**Options** and **Quit**. Hosting an internet game tries to open UDP port 27420 on your router
(UPnP / NAT-PMP); if that fails, forward it by hand.

- **Windows:** use the MSI installer; it adds Start menu shortcuts for flat screen and VR.
- **macOS:** flat screen only. The build is unsigned, so run
  `xattr -dr com.apple.quarantine` on the unpacked folder before the first start.
- **Steam Frame:** copy the aarch64 AppImage to the Frame, `chmod +x` it, start SteamVR and
  run it (from the desktop or as a non-Steam game).

## How to play

Matches last 10 minutes. Score points for your team:

| Action | Points |
|---|---|
| Tag out an enemy (3 hits) | 100 |
| Hit the enemy team's target | 100 |
| Assist (your hit, a team mate's tag-out within 10 s) | 50 |
| Defend (tag-out near your own targets) | +25 |
| Megatarget | 1000 |

In the last minute every point counts double.

- After a hit you're briefly immune and can't shoot. After respawning you're protected until
  you fire or the protection runs out.
- Your own spawn pads heal you and refill your blaster; the white reload stations in the
  middle refill it too.
- The enemy targets hang under a roof deep in their half and can only be hit from behind or
  from the level around them.

### Megatarget mode

The default mode (`--mode classic` for plain target play). The megatarget sits in a sealed
room in the middle, behind forcefields that tag out anyone who walks in. Hit both secondary
targets at the arena's edges to pass the forcefields for 60 s, then hit the megatarget. The
secondary targets are guarded by turrets; shoot the yellow target above a turret to switch it
off for 15 s.

### Arena features

- **Mirrors** bounce a bolt once, so you can bank shots around corners.
- **Jump pads** throw you up to the upper level.
- **Laser gates** switch on and off; walking through one while it's on counts as a hit.
- **Arena events** strike every few minutes: a **Blackout** turns out the lights for 20 s,
  an **Overdrive** respawns every missing pickup at once.

### Pickups

Walk over a pickup to take it. You lose powerups when you're tagged out. Each powerup spot rolls
a weapon, a gadget or a power each time it comes back. You hold one special weapon and carry one
gadget at a time; a new one replaces the old.

| Pickup | Effect |
|---|---|
| Laser grenade | Throw it; after 3 s it spins and fires bolts in all directions, hitting everyone (including you). |
| Rapid fire | The blaster fires twice as fast. |
| Shield | Activate to become immune to bolts for 10 s. |
| Shotgun | Replaces your blaster: 10 short-range bolts per shot. |
| Charge gun | Replaces your blaster: hold the trigger to charge a beam that pierces players and deals two hits. |
| Ricochet rifle | Replaces your blaster: slower-firing bolts that bounce off every wall up to 3 times (but not off targets). |
| Prism lance | Replaces your blaster: a slow marksman rifle whose shot hits instantly; a head hit counts as two. Zoom with Z / mouse button 5 (VR: a laser sight). |
| Cloak | Nearly invisible for 100 s; you flicker into view when you fire or are hit. |
| Barrier | Put up a wall of light for 8 s that blocks enemy bolts but lets your team's through. |
| Mirror | Put up a mirror for 15 s that turns back every bolt, anyone's. |
| Pulse sensor | Throw it; it sticks where it lands and for 20 s marks every enemy within 10 m for your team, through walls and cloaks. |
| Speed boost | Run 50% faster for 15 s. |

### Controls

Flat-screen keys can be rebound in Options. The in-game menu shows the current bindings and a
summary of the rules.

| | VR | Flat screen |
|---|---|---|
| Move | Left stick, or walk in your room | WASD (Shift: walk) |
| Turn / look | Right stick, your head | Mouse |
| Jump | A or X | Space |
| Dash | Click the left stick | Left Alt or mouse button 4 |
| Crouch | Crouch, or pull the right stick down | Left Ctrl or C |
| Draw blaster | Grip at the holster on your hip | Always in hand |
| Fire | Trigger | Left mouse button |
| Grenade | Hold B / Y, throw, let go | Hold G or right mouse button, let go |
| Shield | Trigger of the free hand | Q |
| Gadget (barrier ...) | Grip of the free hand | E |
| Zoom (prism lance) | - | Z or mouse button 5 |
| Scores | In-game menu, or look at your wrist | Hold Tab |
| Menu | Menu button, or hold B / Y | Esc |

Seated VR players can use **Calibrate height** in Options to play at full height.

## Command line options

| Option | Description |
|---|---|
| `--flat` | Start on the flat screen |
| `--vr` | Start in VR (any OpenXR headset) |
| `--gl-vr` | Start in VR with the foveated GL renderer (Steam Frame, Linux; needs `--features glvr`) |
| `--name NAME` | Your player name |
| `--seed N` | Arena seed; starts a match straight away |
| `--bots N` | Number of bots; starts a match straight away |
| `--difficulty easy\|normal\|hard` | Bot difficulty |
| `--arena small\|medium\|large\|random` | Arena size (`random` picks one per match) |
| `--mode classic\|megatarget` | Game mode (default `megatarget`) |
| `--match-time S` | Match length in seconds (local games) |
| `--host [PORT]` | Host a game on your network |
| `--public` | With `--host`: list the game on the internet |
| `--server-name NAME` | Name of the hosted game |
| `--connect HOST[:PORT]` | Join a game |
| `--left-handed` | VR: holster on the left hip |
| `--autopilot` | A bot plays for you (testing) |
| `--screenshot FILE --after T1,T2` | Save screenshots at the given times (s) |
| `--quit-after S` | Quit after S seconds |

Without `--flat`, `--vr` or `--gl-vr`, the game starts in VR when a headset is available and on
the flat screen otherwise.

## Development

```
cargo test --workspace                      # simulation, arena, transport, networking
cargo test -p lt-net --test portmap -- --ignored --nocapture   # test port mapping on this router
tools/models/build_all.sh                   # rebuild and check the models
tools/frame.sh                              # build, deploy to and launch on a Steam Frame
```

- Flat screen without a display: run inside a headless sway (`WLR_BACKENDS=headless`) with
  `--autopilot --screenshot shot.png --after 5,10`.
- VR without a headset: Monado's service with `XRT_COMPOSITOR_NULL=1` and `XR_RUNTIME_JSON`
  pointing at `openxr_monado-dev.json`; `LT_XR_SHOT=8` saves the eye images.

Debug environment variables:

| Variable | Effect |
|---|---|
| `LT_DEV="6:menu,10:play-hard,14:hit"` | Scripted actions at given times (see `dev_script` in `src/session.rs`) |
| `LT_MENU=top\|botmatch\|multiplayer\|internet\|lan\|host-lan\|host-internet` | Open the start menu at that screen |
| `LT_MASTER=URL` | Use another master server |
| `LT_VR_MENU=1\|options` | Open the VR menu / options |
| `LT_SCORES=1` | Show the flat-screen scores |
| `LT_VR_FAKE_HANDS=1\|wrist` | Fake VR controllers |
| `LT_HOLSTER=x,y,z` | Holster position from the eyes (default `0.23,-0.58,-0.07`) |
| `LT_GUN_TILT=DEG` | Blaster tilt in the hand (default 30) |
| `LT_LEFT_HANDED=1` | Same as `--left-handed` |

### Builds and releases

CI (`.github/workflows/build.yml`) builds, tests and packages every push and pull request:
Linux x86_64/aarch64 tarballs and AppImages, a Windows zip and MSI, a macOS Apple silicon zip,
and the dedicated server (`lt-server`) and master server (`lt-master`) for Linux. `/build` in a
pull request comment replies with download links; pushing a `v*` tag publishes a GitHub
release.

The packaging scripts also run locally on Linux:

```
tools/openxr_loader.sh [--arch aarch64]          # the OpenXR loader, into target/openxr-ARCH
tools/appimage.sh --build                        # x86_64 AppImage into dist/
tools/appimage.sh --arch aarch64 --build         # Steam Frame AppImage, cross-compiled
tools/package.sh game x86_64-unknown-linux-gnu --loader target/openxr-x86_64   # tarball
```
