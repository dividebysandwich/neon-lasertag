# Neon Lasertag

Red vs Blue lasertag in procedurally generated neon arenas, for VR headsets (OpenXR) and the
flat screen. Built with Bevy.

<img width="1916" height="1051" alt="image" src="https://github.com/user-attachments/assets/1a696844-08a1-4cbf-84e2-67545a2d226c" />

## Playing the game

**Rules.** Score by tagging out enemy players (100 points) and by hitting the enemy team's
targets (100 points per hit). 

Hitting an enemy whom a team mate tags out within 10 s is an assist (50 points), and tagging
out an enemy within 12 m of your own team's targets (Megatarget
mode: of your team's doorway into the megatarget's room) earns 25 points more for defending.

A player is tagged out by the third hit; after every hit they can't shoot or be hit for 3s. After
respawning nobody can hit them for 1.5 s, or until they fire, throw a grenade or put up a
shield or barrier (they blink meanwhile).
Stepping onto your own team's spawn pads heals all hits
.
The targets are small glowing cores hanging from a roof over the upper level, deep in each team's half.
They can only be hit from the rear quarter of the arena behind them, from the upper level around them or from
the ground below and behind them.

**Megatarget mode** (`--mode megatarget`). Two secondary targets hang at the arena's left and right edges near the 
center line. Each is guarded by two turrets above the doorways, which shoot at every player in front of them.
Hitting the yellow deactivation target above a turret switches it off for 15s.

In the middle is a closed room with a doorway on each team's side, each closed by a shimmering
forcefield that stops bolts and grenades and tags out anyone who walks into it. Hit both
secondary targets (100 points each) and for 60 s you may pass through the forcefields (you tick
like a clock meanwhile, faster in the last 10 s, and everyone hears it).
Inside, the megatarget hangs from the ceiling over a shaft of light. Hitting
it scores 1000 points and sends you straight back to your base with full health and ammo.

**Arena events.** Two to three minutes into a match, and every two to three minutes after
that (never in the last 75 s), something happens to the whole arena, announced with a banner
(VR: floating in front of you) and a sound: a **Blackout** puts the arena's lights out for 20 s
(the city around it too; bolts, players, pickups, targets, jump pads, laser gates and reload
stations still glow, every shot lights up the walls' neon for a few meters around it as it
flies, where it's fired and where it strikes, and the bots only see 12 m), or an **Overdrive** brings back every
grenade and powerup missing from the arena at once. In the last minute every point counts
double (the clock shows 2x).

**What the screen tells you.** In your first matches a few hints (once each) say what to do, how to heal and refill and where the scores are. The in-game menu lists the controls (as bound) and the rules in short, and in VR the scores too. Banners say what an arena event does, what a pickup you take does
and how to use it (the first three times of each kind), what your Megatarget progress means, and
when a powerup runs out; a forcefield you may not pass yet warns you when you come close. Under
the clock (VR: on the wrist) a blackout counts down and 2x shows. Your powerups sit as chips in
their colours above your status panel (VR: icons on the wrist), the timed ones with a bar
draining. Tagged out, you see who did it (in their team's colour), with what and how many hits they had left; the flat screen's view turns to them and follows them, and in VR an arrow points to them with how far away they are. Your hits on enemies mark the crosshair (VR: a tick in the gun hand), the points you
score rise beside it (VR: over the wrist), and the blaster warns when it runs low. A turret
switched off shows a bar draining under its target.

Matches last 10 minutes, after a 3-2-1 countdown (you can move but not fire). A dedicated
server's first match begins with a 30 s warmup (WARMUP under the clock) that doesn't count; it
ends early once two people are in. Then the results show (each player's
score, tags, times tagged out, assists, accuracy and target hits, and up to four awards:
Sharpshooter for the best accuracy, Team player for the most assists, Defender, Grenadier and
Ghost for the most defends, grenade tag-outs and tag-outs while cloaked, Target hunter for the
most target hits and Untouchable for being tagged out least among those with three tag-outs or
more); a local game goes on to the next
match on a new arena after 30 s (or at once with Next match), a server opens its next arena
itself.

Players are hit on their floating parts: the head (one sphere for every animal; ears and
antlers don't count), the chest, the gloves and the blaster.

**Mirrors.** Some wall faces in the middle of the arena are mirrors: dark glass in a lavender
frame with glinting streaks across it. A bolt (any bolt: blasters', grenades', turrets')
bounces off a mirror once, carrying on as if from there, so you can bank a shot round a corner;
a second mirror stops it. The glints flash in the bolt's color and the mirror pings.

**Jump pads.** Mint rings on the ground, with chevrons climbing the way they throw: step onto
one and it throws you up onto the upper floor a few meters on, peaking a little over a meter
above it (you can steer a little in the air). Every pad's flight is checked when the arena is
made; the bots take them too.

**Laser gates.** Some gaps in the walls on the ground have three beams across them between
glowing posts, in the color of the zone they stand in. They switch on for 2 s and off for
2.5 s, humming while on and flickering for half a second before they come on; walking into
one while it's on is a hit (unless your shield is on, or you're still recovering from the last
hit). Bolts and grenades fly through. The bots wait for a gate to go off.

**Jumping and dashing.** A jump is a hop of about 0.8 m, onto a crate or over a low wall. A
dash throws you 3 m along the way you move (or face, standing still) in a quarter of a second,
from the ground only, at most once every 2 s; the panel (flat screen) says when it's ready, and
a dashing player leaves a short trail in their team's colour.

**Blaster.** Three shots a second at most, 50 shots in a full blaster. It refills while you
stand on your own spawn pads or on one of the two white reload stations in the middle of the
arena (25 shots a second).

**Laser grenade.** Nobody starts with one: grenades appear at random spots in the arena (four
at a time, two in each half; a taken one comes back somewhere else after 8 s to 16 s), and
walking over one picks it up. You carry one at most. Thrown, it bounces around (in VR it flies three times as fast as your
hand threw it, so a small swing in a small room carries across the arena). After 3 s it
jumps up and spins for 4 s, shooting white bolts in all
directions. They hit everyone, team mates and the thrower included, and can hit the same
player twice. A grenade in flight trails white sparks inside a slowly pulsing white glow, which
grows and pulses faster while it hovers and fires.

**Powerups.** Each kind lies in the arena once in each half, turning over a ring of its color
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
  your team's color stands 1.5 m in front of you (nearer if a wall is closer) for 8 s. It
  stops every bolt but your team's (enemies', grenades', turrets'), which pass through as if
  it weren't there; anyone can walk through it, and grenades fly through. The bots put theirs
  up when they're hit in a fight, and don't shoot into enemy barriers.
- **Speed boost** (lime, an arrow with speed lines): for 15 s you run half as fast again
  (stick and keys; walking in your room is as it was), leaving a short trail of lime motes.

| | VR | Flat screen |
|---|---|---|
| move | left stick (where you look, or where your left hand points: an option), or walk in your room | WASD (Shift: slower) |
| dash | click the left stick (not on the Steam Frame yet) | Left Alt or mouse button 4 |
| jump | A or X (a hop of about 0.8 m; it still works just after you walk off an edge, and a press just before landing jumps on landing) | Space |
| crouch | crouch, or pull the right stick down (again: stand up); you move at half speed | Left Ctrl or C (hold; lowers you behind half walls, half speed) |
| turn / look | right stick (smooth turn), your head | mouse |
| blaster | grip it at the holster on your right hip (one hand at a time); let go to holster it | always in hand |
| fire | trigger | left mouse button |
| grenade | hold B (or Y), throw, let go | hold G or the right mouse button (shows the throw's arc and where it goes off), let go |
| shield (powerup) | trigger of the hand without the blaster | Q |
| barrier (powerup) | grip of the hand without the blaster, away from the holster | E |
| scores | in the in-game menu | hold Tab |
| menu | menu button (left Touch controller), or hold B / Y for a second without a grenade; point and pull the trigger | Esc |

## Command Parameters

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

**The start menu** (flat screen and VR alike): Botmatch (the bots' number and difficulty, the
arena's size, the mode), Multiplayer, Options (VR: volumes, the blaster's tilt, the holster's height and side, vibration, whether the stick moves you where you look or where your left hand points, and Calibrate height: sit or stand upright and press it to play at full height, e.g. seated; also in the in-game menu; flat screen: volumes, mouse, field of view, crouch held or toggled, head bob and blaster sway, the keys) and Quit. Multiplayer offers Internet
(the master server's list of servers, join one, or host an internet game) and LAN (the games
found on your network, found by their broadcasts on UDP port 27421, join one, or host a LAN
game); on the flat screen both also join a typed address. Hosting asks for the server's name
(flat screen), the bots, the arena and the mode. A hosted game is announced on the LAN; an
internet one is also listed on the master server, and its UDP port (27420) is opened on the
router with UPnP, or NAT-PMP. If the router does neither, the game is hosted and listed anyway,
and the in-game menu says to forward the port by hand.

Other flags: `--name NAME`, `--host [PORT]` (play and let others on your network join),
`--public` (with `--host`: on the internet, listed on the master server, the port opened on the
router), `--server-name NAME` (the hosted game's name), `--connect HOST[:PORT]`,
`--difficulty easy|normal|hard` (the bots, also in the start menu), `--arena small|medium|large|random`
(the arenas' size in local and hosted games, also in the start menu; random picks one for every
match), `--mode classic|megatarget` (what local and hosted games are played for, megatarget unless chosen, also in the start
menu), `--left-handed` (VR: the
holster on the left hip, as the option; or `LT_LEFT_HANDED=1`), `--autopilot` (a bot plays you, for testing),
`--match-time S` (local games), `--screenshot FILE --after 5,10`, `--quit-after S`.
Testing: `LT_DEV="6:menu,8:leave,10:play-hard,14:hit,20:next"` scripts menu actions and hits (`options`
opens the options screen, `arc` shows the throw arc, `target` hits a team target, `hit-them` and `tag-them` show and sound as if you hit or tagged out the nearest enemy, `tag-me` has the nearest enemy tag you out (local games);
`give-KIND` gives you a pickup's contents (`give-all-KIND` everyone, `take-KIND` as if you walked over one, announced) and `drop-KIND` lays one in front of you, KIND
`grenade`, `rapid`, `shield`, `shotgun`, `cloak`, `speed`, `charge`, `barrier`; `fire=S` holds the trigger down for S s, `barrier` puts up a carried barrier; `blackout` and `overdrive` start those arena events, `double` jumps to the last minute's double points; `shields-up` switches everyone's shield on, `summon` (or `summon=M`) puts the
nearest other player 4 m (M m) in front of you; Megatarget mode: `power` hits both secondary targets, `turret-off` switches the first turret off, `warp-door`, `warp-room`, `warp-turret` and `warp-pocket` move
you there; `warp-mirror` puts you before the nearest mirror, `warp-pad` (or `warp-pad=M`) 1 m (M m) before
the nearest jump pad, `warp-gate` before the nearest laser gate; `yaw=DEG` and `pitch=DEG` turn the flat screen's view),
`LT_MENU=top|botmatch|multiplayer|internet|lan|host-lan|host-internet` opens the start menu at
that level, `LT_MASTER=URL` asks another master server, `LT_VR_MENU=1` opens the VR menu, `LT_SCORES=1` shows the flat screen's scores as if Tab were held, `LT_VR_FAKE_HANDS=1` fakes controllers (`wrist`: the free hand held up, showing its wrist display), `LT_HOLSTER=x,y,z`
moves the holster (from the eyes; default `0.23,-0.58,-0.07`). `LT_GUN_TILT=DEG` turns the
blaster nose down in the hand by that much from the controller's aim (instead of the option,
default 30), for a more upright handle. `LT_VR_MENU=options` opens the VR options.


## Testing

```
cargo test --workspace                      # simulation, arena properties, transport, networking
cargo test -p lt-net --test portmap -- --ignored --nocapture   # open (and list) ports on this router
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

- **clippy**: `cargo clippy --workspace` on Linux (`glvr`); warnings reported, not fatal.
- **build**: per platform, a release build of the workspace, its packages, then
  `cargo test --release --workspace` with the same flags (so the tests reuse the build's
  compiled crates). Linux uses `glvr`, Windows `xr`, macOS neither (no OpenXR there); none uses
  `dev` (bevy/dynamic_linking is for development only). The rust caches are saved from `main`
  only. The packages:
  - Linux x86_64 and aarch64: `lasertag-VERSION-TARGET.tar.gz` (the game, `assets/`, and the
    OpenXR loader `libopenxr_loader.so` beside the game, which its RUNPATH `$ORIGIN` finds) and
    `lasertag-VERSION-ARCH.AppImage`. The aarch64 one is built on GitHub's arm64 runner.
  - Windows x64: `lasertag-VERSION-x86_64-pc-windows-msvc.zip` (`--vr` works; the loader is
    linked in).
  - macOS Apple silicon: `lasertag-VERSION-aarch64-apple-darwin.zip`, flat screen only. It is
    unsigned: `xattr -dr com.apple.quarantine` the unpacked folder before the first start.
- **servers**: `lt-server-VERSION-...tar.gz` (dedicated server; announced on the LAN unless
  `--no-lan`, listed on the master server unless `--private`) and `lt-master-VERSION-...tar.gz`
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
