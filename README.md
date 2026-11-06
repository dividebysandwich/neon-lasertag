# Neon Lasertag

Red vs Blue lasertag in procedurally generated neon arenas, for VR headsets (OpenXR) and the
flat screen. Built with Bevy 0.19.

## Playing

```
cargo run --release                         # VR if there's a headset, else the flat screen
cargo run --release -- --flat               # the flat screen, start menu
cargo run --release -- --seed 42 --bots 7   # straight into a match against bots
cargo run --release -- --vr                 # any OpenXR headset (bevy_mod_openxr, wgpu, bloom)
cargo run --release --features glvr -- --gl-vr   # foveated GL renderer (Steam Frame, Linux)
```

Without `--vr`, `--gl-vr` or `--flat` the game starts in VR on a Steam Frame (`--gl-vr`) or
while a VR compositor runs (SteamVR's vrcompositor or Monado's service: `--vr`), and on the
flat screen otherwise (or when the headset can't be used).

Other flags: `--name NAME`, `--host [PORT]` (play and let others join), `--connect HOST[:PORT]`,
`--difficulty easy|normal|hard` (the bots, also in the start menu), `--arena small|medium|large|random`
(the arenas' size in local and hosted games, also in the start menu; random picks one for every
match), `--mode classic|megatarget` (what local and hosted games are played for, also in the start
menu), `--left-handed` (VR: the
holster on the left hip; or `LT_LEFT_HANDED=1`), `--autopilot` (a bot plays you, for testing),
`--match-time S` (local games), `--screenshot FILE --after 5,10`, `--quit-after S`.
Testing: `LT_DEV="6:menu,8:leave,10:play-hard,14:hit,20:next"` scripts menu actions and hits (`options`
opens the options screen, `arc` shows the throw arc, `target` hits a team target;
`give-KIND` gives you a pickup's contents (`give-all-KIND` everyone) and `drop-KIND` lays one in front of you, KIND
`grenade`, `rapid`, `shield`, `shotgun`, `cloak`, `speed`, `charge`, `barrier`; `fire=S` holds the trigger down for S s, `barrier` puts up a carried barrier; `shields-up` switches everyone's shield on, `summon` (or `summon=M`) puts the
nearest other player 4 m (M m) in front of you; Megatarget mode: `power` hits both secondary targets, `warp-door`, `warp-room`, `warp-turret` and `warp-pocket` move
you there; `warp-mirror` puts you before the nearest mirror, `warp-pad` (or `warp-pad=M`) 1 m (M m) before
the nearest jump pad, `warp-gate` before the nearest laser gate; `yaw=DEG` and `pitch=DEG` turn the flat screen's view),
`LT_VR_MENU=1` opens the VR menu, `LT_VR_FAKE_HANDS=1` fakes controllers, `LT_HOLSTER=x,y,z`
moves the holster (from the eyes; default `0.23,-0.58,-0.07`). `LT_GUN_TILT=DEG` turns the
blaster nose down in the hand by that much from the controller's aim (default 15), for a more
upright handle.

**Rules.** Score by tagging out enemy players (100 points) and by hitting the enemy team's
targets (100 points per hit). A player is tagged out by the third hit; after every hit they
can't shoot, and can't be hit, for 3 s (they can still move). Tagged-out players wait 5 s to
15 s to respawn at their base, longer the further from it they were tagged. Stepping onto your
own team's spawn pads heals all hits. The targets are small
glowing cores hanging from a roof over the upper level, deep in each team's half, in a cell
walled on three sides on both floors, with chevrons running in towards them: they can only be
hit from the rear quarter of the arena behind them, from the upper level around them or from
the ground below and behind them.
**Megatarget mode** (`--mode megatarget`). The arena is half as big again each way and has no
team targets. Two secondary targets hang at the arena's left and right edges, across the centre
line, on the back of a wall two storeys high: to hit one you have to get into the pocket behind
that wall through a doorway at either end of it. Each is guarded by two turrets on the wall's
outer face, above the doorways, which shoot at every player in front of them (16 m); hitting the
yellow deactivation target above a turret switches it off for 15 s. In the middle stands a
closed room two storeys high with a doorway on each team's side, each closed by a shimmering
forcefield that stops bolts and grenades and tags out anyone who walks into it. Hit both
secondary targets (100 points each) and for 60 s you may pass through the forcefields (you tick
like a clock meanwhile, faster in the last 10 s, and everyone hears it): inside, the megatarget
hangs from the ceiling over a shaft of light, with chevrons climbing the walls towards it. Hitting
it scores 1000 points and sends you straight back to your base with full health and shots. Tagged
out, or out of time, you start over; out of time inside the room (or in its doorway), you are
tagged out.

Matches last 10 minutes. Then the results show (each player's
score, tags, times tagged out, accuracy and target hits); a local game goes on to the next
match on a new arena after 30 s (or at once with Next match), a server opens its next arena
itself.

Players are hit on their floating parts: the head (one sphere for every animal; ears and
antlers don't count), the chest, the gloves and the blaster.

**Mirrors.** Some wall faces in the middle of the arena are mirrors: dark glass in a lavender
frame with glinting streaks across it. A bolt (any bolt: blasters', grenades', turrets')
bounces off a mirror once, carrying on as if from there, so you can bank a shot round a corner;
a second mirror stops it. The glints flash in the bolt's colour and the mirror pings.

**Jump pads.** Mint rings on the ground, with chevrons climbing the way they throw: step onto
one and it throws you up onto the upper floor a few metres on, peaking a little over a metre
above it (you can steer a little in the air). Every pad's flight is checked when the arena is
made; the bots take them too.

**Laser gates.** Some gaps in the walls on the ground have three beams across them between
glowing posts, in the colour of the zone they stand in. They switch on for 2 s and off for
2.5 s, humming while on and flickering for half a second before they come on; walking into
one while it's on is a hit (unless your shield is on, or you're still recovering from the last
hit). Bolts and grenades fly through. The bots wait for a gate to go off.

**Blaster.** Three shots a second at most, 50 shots in a full blaster. It refills while you
stand on your own spawn pads or on one of the two white reload stations in the middle of the
arena (25 shots a second).

**Laser grenade.** Nobody starts with one: grenades appear at random spots in the arena (four
at a time, two in each half; a taken one comes back somewhere else after 8 s to 16 s), and
walking over one picks it up. You carry one at most. Thrown, it bounces around. After 3 s it
jumps about a metre up, then hovers and spins for 4 s, spraying white bolts in all
directions, one in five of them aimed at a player it can see (within 20 m). They hit everyone, team mates and the thrower included, and can hit the same
player twice. A grenade in flight trails white sparks inside a slowly pulsing white glow, which
grows and pulses faster while it hovers and fires.

**Powerups.** Each kind lies in the arena once in each half, turning over a ring of its colour
(a taken one comes back somewhere else in that half after 25 s to 45 s); walk over one to take
it. You lose them all when you're tagged out.
- **Rapid fire** (yellow, two chevrons): the blaster fires twice as fast.
- **Shield** (cyan): carried until you switch it on (Q; VR: the trigger of the hand without the
  blaster), then no bolt hurts you for 10 s. A shielded player stands in a beam of cyan light
  from above (flickering in its last two seconds); bots and turrets don't waste shots on them.
- **Shotgun** (orange): replaces your blaster. Every shot (one of the 50) fires 10 bolts in a
  slight spread that fly only 14 m. Taking one also fills it up.
- **Cloak** (purple, a cape): for 100 s you can't be seen from further than 7 m, and closer you
  are only a faint shimmer (bots and turrets only notice you within 3 m, though they still hear
  you). You show for half a second whenever you fire or are hit.
- **Charge gun** (pink, a lightning bolt): replaces your blaster, with coils round its barrel.
  A tap of the trigger fires an ordinary bolt; hold it for a second (the muzzle glows brighter
  and the gun hums higher, VR: in your hand) and let go for a beam that crosses 60 m almost at
  once, flies through every player in its way and is two hits on each, for 5 shots. Getting
  hit while charging stops it. Taking one also fills it up. You carry one special gun at a
  time: while you have the shotgun or the charge gun, the other stays where it lies.
- **Barrier** (emerald, a panel between two posts): carried until you put it up (E; VR: grip
  with the hand without the blaster, away from the holster), then a wall of hard light in
  your team's colour stands 1.5 m in front of you (nearer if a wall is closer) for 8 s. It
  stops every bolt but your team's (enemies', grenades', turrets'), which pass through as if
  it weren't there; anyone can walk through it, and grenades fly through. The bots put theirs
  up when they're hit in a fight, and don't shoot into enemy barriers.
- **Speed boost** (lime, an arrow with speed lines): for 15 s you run half as fast again
  (stick and keys; walking in your room is as it was), leaving a short trail of lime motes.

| | VR | Flat screen |
|---|---|---|
| move | left stick (where you look), or walk in your room | WASD (Shift: slower) |
| crouch | crouch | Left Ctrl or C (hold; lowers you behind half walls, slower) |
| turn / look | right stick (smooth turn), your head | mouse |
| blaster | grip it at the holster on your right hip (one hand at a time); let go to holster it | always in hand |
| fire | trigger | left mouse button |
| grenade | hold B (or Y), throw, let go | hold G or the right mouse button (shows the throw's arc and where it goes off), let go |
| shield (powerup) | trigger of the hand without the blaster | Q |
| barrier (powerup) | grip of the hand without the blaster, away from the holster | E |
| menu | menu button (left Touch controller), or hold B / Y for a second without a grenade; point and pull the trigger | Esc |

Stick and key movement has a little inertia (about a tenth of a second to get up to speed or
to stop); walking in your room doesn't. On the flat screen the view bobs slightly as you walk,
and the blaster sways with your steps and lags a little behind when you turn. In VR the wrist of your free hand shows the hits you have left, whether you carry a grenade,
the powerups you have, the shots left and the respawn countdown. On the back wall of each base hang a score board (Red's score left, Blue's right) and, above it, a board with the time left.
Hits glow red at the edge of the view on the side they came from (on the flat screen: a red
arc around the crosshair), and the controllers vibrate: shots, empty clicks, hits (stronger on
that side), being tagged out, grenades, drawing and holstering. The menu (Esc) leaves the match
running underneath: Resume, Options, Leave to main menu, Quit; VR's offers Resume, New match
(local games), Main menu and Quit. VR's start menu floats in an empty arena under colourful
fireworks (`src/lobby.rs`): Play vs bots or Host game, with the bots' number and skill and the
arenas' size, or Quit. Options (also in the start menu) sets the sound effects' and the music's volume, rebinds the
flat screen's keys and mouse buttons (two per action), sets the mouse sensitivity and inverts
the mouse; they are kept
in `settings.ron` in the config directory (`~/.config/lasertag`, `%APPDATA%\lasertag`; VR uses
the saved volumes too).

## Layout

| path | what |
|---|---|
| `crates/lt-sim` | The game without a renderer: rules (`rules.rs`), arena generation (`level.rs`), modular kit dimensions (`kit.rs`), collision (`collide.rs`), the simulation (`sim.rs`, 60 Hz, authoritative), bots (`bots.rs`: the `Controller` trait and the placeholder `WanderBot`). |
| `crates/lt-net` | UDP transport (reliable and unreliable channels; ported from stealth-mission-89), protocol (CBOR + LZ4), server and client sessions, master server reporting. |
| `crates/lt-server` | Dedicated server: `lt-server --port 27420 --bots 8 --difficulty normal --name NAME [--arena small|medium|large|random] [--mode classic|megatarget] [--seed N] [--private] [--master URL]`. |
| `crates/lt-master`, `lt-master-proto` | Master server list (REST/JSON): `lt-master --bind 0.0.0.0:27500`, with deploy files in `crates/lt-master/deploy`. |
| `src/` | The Bevy client: `game.rs` (local sim or connection, stepped in `FixedUpdate`, client movement prediction), `arena.rs` (kit pieces merged into chunk meshes), `materials.rs` + `assets/shaders/neon.wgsl` (neon accents and glow halos), `actors.rs` (players, bolts, grenades, sparks), `desktop.rs`, `vr.rs` (shared VR input, holster, haptics), `vrui.rs` (VR menus, end panel, hit flash; text from `segtext.rs`), `lobby.rs` (VR start menu scene, fireworks), `xr.rs` (bevy_mod_openxr), `glvr/` (foveated GL renderer), `audio.rs`, `hud.rs`, `sevenseg.rs`, `targets.rs` (team targets), `mega.rs` (the Megatarget mode's pieces), `menu.rs`, `pause.rs` (in-game menu), `endscreen.rs` (results), `session.rs` (next match, leaving a game). |
| `tools/models` | Blender build scripts for the kit, blaster, grenade, avatars and reload station (`build_all.sh`, conventions in its README). |
| `tools/gen_sfx.py` | Sound synthesis (numpy + ffmpeg): `python3 tools/gen_sfx.py`. |
| `tools/gen_spectrum.py` | The music's precomputed spectrum: `python3 tools/gen_spectrum.py`. |
| `tools/frame.sh` | Cross-compile, deploy and run on a Steam Frame over ssh. |

### Arenas

The generator works on a 2 m grid, with two storeys of 3 m: 14 x 28 cells in a small arena,
18 x 34 in a medium one (about 25 % bigger each way) and 21 x 42 in a large one (50 %). Bigger
arenas get more of everything (cover, platforms, holes, doors, panels), in proportion to their
floor; the size travels to clients in the `Rules` with the seed. It builds Red's
half (+z) and turns it half way round the centre for Blue's (so the bases are in opposite
corners). Each half has:

- a small base in a rear corner: 3 x 2 spawn pads, walled off, with a gate on each open side;
- one or two team targets in the rear quarter (`kit::target_box`), each hanging from a roof
  over the upper level, in a cell walled on three sides on both floors and open to the rear;
  neither that cell nor the one behind it has an upper floor, so the target is seen from the
  ground under and behind it and from the upper floor around the opening (at least two places
  on each floor, checked for every arena; drawn by `src/targets.rs`);
- a reload station in the middle third, walled with windows on two sides;
- an upper floor of connected platforms, with holes cut in to shoot up and down through, and
  at least two ramps per half; the upper level always crosses the arena (a walkway joins the
  targets' surroundings to the bridge over the centre line where the platforms don't);
- cover: full walls, walls with windows, half walls, pillars and blocks;
- mirrors (`Level::mirrors`): one to three faces of full walls (more in bigger arenas),
  between the centre line and the targets' rows, each with open floor in front of it to shoot
  from (drawn by `src/features.rs`);
- jump pads (`Level::pads`): one or two (more in bigger arenas) on open ground with nothing
  overhead, each throwing onto the upper floor one to three cells away; the flight is
  simulated when the arena is made, with the doors and panels at both ends of their way, and
  must be clear and land there (`level::pad_flight`). The bots' map has them as one-way links;
- laser gates (`Level::gates`): one or two (more in bigger arenas) across gaps on the ground
  shut off on both sides (by walls, pillars, blocks or the border), never where shutting
  them would fail the arena's checks; on and off by the sim tick alone, like the doors.

Long open lanes are broken up. Every arena is checked before it is used: every base reaches the
centre, the upper floor and the rear of the enemy targets, the upper level leads from one
team's target to the other's without coming down, and each target can be hit from at least
two places on each floor. Each kit piece's numbers live only in
`crates/lt-sim/src/kit.rs`. `tools/models/kit_dims.json` is generated from it
(`LT_WRITE_KIT_DIMS=1 cargo test -p lt-sim kit`), and the Blender kit builder reads that file.

Behind the targets the accents take the colour of the half they stand in (chosen per pixel
by world z). The middle of the arena is split into zones of purple, yellow, green, cyan and
orange, turned round like the arena, and the walls around the reload stations are white; white is
used nowhere else. The zone colours are baked into the merged meshes as vertex colours,
with the brightness: ordinary accents are dimmed and vary from piece to piece (a few are nearly
dark), each places only some of its kit piece's accent shapes, and the floor grid is thin and
faint. The bases, targets and stations glow fully. Slow drifting patches of light and dark move
over the accents, and now and then a streak of light races along them. Inside the arena,
faint shafts of light fall through the holes of the upper floor, low haze drifts over the
floor, motes float in the air, and the reload stations shine a soft column of light onto
their pads (`src/ambience.rs`). Waves of colour run from each base towards the centre, and
target hits and tag-outs send ripples across the walls. The border walls around the arena turn slowly through the rainbow and pulse with the music's bass. Bloom makes them glow in the wgpu
renderers. The GL renderer has no post-processing, because foveation needs a single pass into
the swapchain, so the kit carries additive glow halos that work in every renderer.

### Atmosphere

The sound effects are synthesized by `tools/gen_sfx.py` and play in space around you, fading
gently with distance (`src/audio.rs`); a team target being hit is heard across the whole arena.
Hits throw out sparks and a flash of light; a hit team target lights up its surroundings,
flickering for a second. Hitting an enemy or an enemy target bleeps (and a tag-out plays a little fanfare); Red's and
Blue's blasters sound different.
The music is picked at random from `assets/music`: `menu*.ogg` in the menus, `game*.ogg`
during a match (one after another), and `won*.ogg` or `lost*.ogg` once when a match ends (a
draw counts as lost). Each track's spectrum is worked out ahead
of time by `tools/gen_spectrum.py` (`X.spectrum` beside `X.ogg`: six bands from 20 Hz to
16 kHz and a bass pulse, 50 frames a second) and followed in step with playback
(`src/music.rs`). It drives the two big bar-graph displays above the rear walls and the
pulsing of some of the light cones over the arena. Around the arena stands a procedural city
(`src/scenery.rs`): towers with lit windows, neon bands and halos, and searchlights. All of it
is built from the neon materials so the foveated GL renderer draws it too.
`LT_VIEW=x,y,z,tx,ty,tz` pins the camera (or the headset's rig) for aimed screenshots.

### Bots and multiplayer

The simulation takes the same `PlayerCmd` every tick from local players, network players and
bots (`lt_sim::bots::Controller`). The bots build a graph of the arena when it starts (the
level's walkable cells plus clear diagonals, A* with path smoothing) and rate every spot for
cover from the enemy spawn, the enemy targets it can hit and the own targets it watches. Half
of each team attacks, half defends. Every 10-20 s a bot picks a plan: hold a covered spot,
hunt the nearest enemy it knows of, or (attackers) shoot an enemy target from cover / (defenders)
take another spot covering its own targets. It breaks off to refill when low on shots and to
pick up grenades it sees, and lobs grenades at groups of enemies. Each bot has its own aiming
error and reaction time.

The server is authoritative. Clients send their latest commands, redundantly (`ToServer::Input`).
Each client predicts its own movement and sends the result, and the server accepts it within
the speed limit and its own collision. The server sends snapshots every other tick and events
reliably. A dedicated server lists itself on the master server; hosting from the client and
joining by address or from the menu's list also work.

## Testing

```
cargo test --workspace                      # simulation, arena properties, transport, networking
tools/models/build_all.sh                   # rebuild and check the models
```

- Flat screen without a display: run the game inside a headless sway (`WLR_BACKENDS=headless`)
  with `--autopilot --screenshot shot.png --after 5,10`.
- VR without a headset: Monado's service with `XRT_COMPOSITOR_NULL=1` (simulated HMD), with
  `XR_RUNTIME_JSON` pointing at its `openxr_monado-dev.json`. Run the game with `--vr` or
  `--gl-vr`; `LT_XR_SHOT=8` saves the eye images.
- Steam Frame: `tools/frame.sh` (defaults to `--gl-vr`; `tools/frame.sh -- --vr` for wgpu).

## Builds and releases

CI (`.github/workflows/build.yml`) runs on every push and pull request:

- **test**: `cargo clippy` (warnings reported, not fatal) and `cargo test --workspace` on Linux
  (`glvr`), Windows (`xr`) and macOS (no OpenXR there), all without the `dev` feature.
- **build**: release packages, without `dev` (bevy/dynamic_linking is for development only):
  - Linux x86_64 and aarch64: `lasertag-VERSION-TARGET.tar.gz` (the game, `assets/`, and the
    OpenXR loader `libopenxr_loader.so` beside the game, which its RUNPATH `$ORIGIN` finds) and
    `lasertag-VERSION-ARCH.AppImage`. The aarch64 one is built on GitHub's arm64 runner.
  - Windows x64: `lasertag-VERSION-x86_64-pc-windows-msvc.zip` (`--vr` works; the loader is
    linked in).
  - macOS Apple silicon: `lasertag-VERSION-aarch64-apple-darwin.zip`, flat screen only. It is
    unsigned: `xattr -dr com.apple.quarantine` the unpacked folder before the first start.
- **servers**: `lt-server-VERSION-...tar.gz` (dedicated server) and `lt-master-VERSION-...tar.gz`
  (master server, with `deploy/`) for Linux x86_64.

Every run keeps its packages as workflow artifacts; `/build` in a pull request comment answers
with their download links (`build-PR-links.yml`). Pushing a tag `v*` attaches them all to a
GitHub release.

The packaging scripts run locally too (Linux):

```
tools/openxr_loader.sh [--arch aarch64]          # the OpenXR loader, into target/openxr-ARCH
tools/appimage.sh --build                        # x86_64 AppImage into dist/
tools/appimage.sh --arch aarch64 --build         # Steam Frame AppImage, cross-compiled
tools/package.sh game x86_64-unknown-linux-gnu --loader target/openxr-x86_64   # tarball
```

`tools/appimage.sh` takes `--binary PATH` to package an existing release build instead of
`--build`, and builds the loader when `target/openxr-ARCH` has none. Cross-compiling for aarch64
uses the same arm64 sysroot as `tools/frame.sh` (`target/sysroot-arm64`, made on first use;
needs aarch64-linux-gnu-gcc, bsdtar, cmake). The AppImage's icon is drawn by
`tools/gen_icon.py`; its AppRun and .desktop file are in `packaging/linux`.

**Running on the Steam Frame.** Copy the aarch64 AppImage to the Frame (e.g. `scp` to
`~/Downloads`), `chmod +x` it, start SteamVR and run it, from the desktop or added to Steam as a
non-Steam game. It starts in VR with `--gl-vr` (the foveated GL renderer); other arguments are
passed on, `--vr` picks the wgpu renderer instead and `--desktop` keeps the flat-screen window.
Started without a terminal it logs to `~/.local/state/lasertag/lasertag.log`. The x86_64
AppImage starts the flat-screen menu and takes the same arguments. For development,
`tools/frame.sh` builds, deploys over ssh and launches in one go.
