//! `--gl-vr`: the headset drawn with our own OpenGL renderer instead of Bevy's, so the Steam
//! Frame's eye tracked foveation (Valve's density map layers, see `foveation.rs`) applies: it
//! acts on OpenGL-over-Zink when the frame is drawn in one pass straight into the swapchain
//! image. Ported from stealth-mission-89's src/glvr.
//!
//! Bevy runs without its renderer and window. A render thread owns the OpenGL context and the
//! headset's frames: it waits for each frame, tells the main thread to update the game (with
//! the head and controllers of that frame in `vr::XrInput`), and meanwhile draws the last
//! update's draw list (`extract.rs`) with the eyes' poses of this frame, so updating and
//! drawing overlap as with Bevy's pipelined rendering. The shared VR play (src/vr.rs) places
//! the tracking space in the arena through the rig (`vr::XrRig`).
//!
//! Environment: `LT_GL_FOVEATION` = off, low, medium (default) or high; `LT_FDM_ENV="K=V;..."`;
//! `LT_GL_MSAA` = 0, 2 (default) or 4; `LT_GL_DRIVER=native` draws with Mesa's native driver
//! instead of Zink; `LT_GL_DELAYED=1` shows each image a frame after it is
//! drawn; `LT_XR_SHOT=<secs>` saves the eyes' images (xr-eye0.png, xr-eye1.png).

mod controllers;
mod egl;
mod extract;
mod foveation;
mod headset;
mod render;

use bevy::app::AppExit;
use bevy::asset::AssetEvent;
use bevy::prelude::*;
use bevy::window::ExitCondition;
use extract::{DrawList, Extractor, Eyes};
use render::AssetMsg;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::vr::{VrPlugin, XrInput, XrRig};

/// Mesa's GL version caps, which Steam's gamescope session sets (4.3) for games launched from it:
/// Mesa then refuses the 4.5 core context glvr needs.
const GL_VERSION_OVERRIDES: [&str; 2] = ["MESA_GL_VERSION_OVERRIDE", "MESA_GLSL_VERSION_OVERRIDE"];

/// Mesa's choice of driver.
const DRIVER_OVERRIDE: &str = "MESA_LOADER_DRIVER_OVERRIDE";

/// EGL_PLATFORM_SURFACELESS_MESA's extension.
const SURFACELESS: &str = "EGL_MESA_platform_surfaceless";

/// Whether an OpenXR runtime with OpenGL through EGL (XR_MNDX_egl_enable) and an EGL with the
/// surfaceless platform are there, without creating an instance (which would start SteamVR) or
/// starting a driver. (OpenGL 4.5 with GL_OVR_multiview2 is found out on the render thread.)
/// Said on stderr: Bevy's log isn't set up yet.
pub fn runtime_available() -> bool {
    // SAFETY: loads the OpenXR loader library.
    let Ok(entry) = (unsafe { openxr::Entry::load() }) else {
        eprintln!("glvr: no OpenXR loader");
        return false;
    };
    match entry.enumerate_extensions() {
        Ok(e) if e.mndx_egl_enable && e.khr_opengl_enable => {}
        Ok(_) => {
            eprintln!("glvr: the OpenXR runtime has no OpenGL through EGL (XR_MNDX_egl_enable)");
            return false;
        }
        Err(e) => {
            eprintln!("glvr: no OpenXR runtime ({e})");
            return false;
        }
    }
    // SAFETY: libEGL's functions with EGL's signatures.
    let egl = match unsafe { khronos_egl::DynamicInstance::<khronos_egl::EGL1_5>::load_required() } {
        Ok(egl) => egl,
        Err(e) => {
            eprintln!("glvr: no EGL ({e})");
            return false;
        }
    };
    // (The client extensions: asked of no display, no driver is started.)
    let surfaceless = egl
        .query_string(None, khronos_egl::EXTENSIONS)
        .is_ok_and(|s| s.to_string_lossy().split(' ').any(|e| e == SURFACELESS));
    if !surfaceless {
        eprintln!("glvr: EGL has no {SURFACELESS}");
    }
    surfaceless
}

/// Asks for foveation, OpenGL on Zink and drops Mesa's GL version caps before anything starts
/// (the environment is read when Zink makes its Vulkan instance and EGL its context): the line
/// to log.
pub fn before_start() -> String {
    let mut line = foveation::enable();
    // Valve's layers foveate Vulkan render passes: OpenGL drawn through Zink (as on the Steam
    // Frame, where it is the only OpenGL). Mesa's native drivers (radeonsi) also lack
    // GL_OVR_multiview2. LT_GL_DRIVER=native keeps the system's driver.
    let native = std::env::var("LT_GL_DRIVER").is_ok_and(|v| v == "native");
    if !native && std::env::var_os(DRIVER_OVERRIDE).is_none() {
        // SAFETY: called from `add_plugins`, before any other thread starts.
        unsafe { std::env::set_var(DRIVER_OVERRIDE, "zink") };
        line += &format!("; {DRIVER_OVERRIDE}=zink");
    }
    for key in GL_VERSION_OVERRIDES {
        if let Some(value) = std::env::var_os(key) {
            // SAFETY: called from `add_plugins`, before any other thread starts.
            unsafe { std::env::remove_var(key) };
            line += &format!("; dropped {key}={}", value.to_string_lossy());
        }
    }
    line
}

/// The app's plugins for `--gl-vr`; false (nothing added) if there is no runtime or EGL for it.
/// Called before any thread starts (the environment is set for the foveation).
pub fn add_plugins(app: &mut App, asset_root: &str) -> bool {
    if !runtime_available() {
        return false;
    }
    let fovea = before_start();
    // (What the GPU would decode: no textures are drawn here. Said before the image plugins
    // look for it, as RenderPlugin would with a GPU.)
    app.insert_resource(bevy::image::CompressedImageFormatSupport(bevy::image::CompressedImageFormats::NONE))
        .add_plugins(
            DefaultPlugins
                .set(bevy::render::RenderPlugin {
                    render_creation: bevy::render::settings::WgpuSettings { backends: None, ..default() }.into(),
                    ..default()
                })
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(AssetPlugin { file_path: asset_root.to_string(), ..default() })
                .disable::<bevy::winit::WinitPlugin>(),
        )
        // Despawning an entity synced to the render world (lights) records it for the render
        // world, which isn't there: without the record keeping that panics.
        .add_plugins(bevy::render::sync_world::SyncWorldPlugin)
        .add_plugins((VrPlugin, GlVrPlugin(fovea)));
    true
}

/// Meshes changed or gone since the renderer last looked.
#[derive(Resource, Default)]
struct DirtyMeshes(Vec<AssetId<Mesh>>);

/// The ear: kept at the head in the world.
#[derive(Component)]
struct Listener;

struct GlVrPlugin(String);

impl Plugin for GlVrPlugin {
    fn build(&self, app: &mut App) {
        info!("glvr: {}", self.0);
        app.init_resource::<DirtyMeshes>()
            .add_systems(Startup, spawn_rig)
            .add_systems(PostUpdate, follow_head.before(TransformSystems::Propagate))
            .add_systems(Last, collect_dirty)
            .set_runner(runner);
    }
}

/// The rig (the tracking space in the arena, placed by vr.rs) and the listener.
fn spawn_rig(mut commands: Commands) {
    let rig = commands.spawn((Name::new("XR rig"), Transform::IDENTITY, Visibility::default())).id();
    commands.insert_resource(XrRig(rig));
    commands.spawn((Name::new("XR listener"), Listener, SpatialListener::new(0.2), Transform::IDENTITY));
}

/// The listener at the head in the world: the rig's transform times the head's tracking pose.
fn follow_head(
    input: Res<XrInput>,
    rig: Option<Res<XrRig>>,
    rigs: Query<&Transform, Without<Listener>>,
    mut ears: Query<&mut Transform, With<Listener>>,
) {
    let (Some(head), Some(rig)) = (input.head, rig) else { return };
    let Ok(r) = rigs.get(rig.0) else { return };
    for mut t in &mut ears {
        *t = *r * head;
    }
}

fn collect_dirty(mut events: MessageReader<AssetEvent<Mesh>>, mut dirty: ResMut<DirtyMeshes>) {
    for e in events.read() {
        match e {
            AssetEvent::Modified { id } | AssetEvent::Removed { id } | AssetEvent::Unused { id } => dirty.0.push(*id),
            _ => {}
        }
    }
}

/// `LT_GL_MSAA`: samples a pixel (0, 2 or 4; default 2).
fn samples() -> i32 {
    std::env::var("LT_GL_MSAA").ok().and_then(|v| v.parse().ok()).unwrap_or(2)
}

fn runner(mut app: App) -> AppExit {
    match run(&mut app) {
        Ok(exit) => exit,
        Err(e) => {
            error!("glvr: {e}");
            AppExit::error()
        }
    }
}

/// The head between the eyes `eyes`, facing as the first.
fn head_of(eyes: &[openxr::View]) -> Option<Transform> {
    let first = eyes.first()?;
    let middle = eyes.iter().map(|v| render::pose(v).0).sum::<Vec3>() / eyes.len() as f32;
    Some(Transform::from_translation(middle).with_rotation(render::pose(first).1))
}

fn run(app: &mut App) -> Result<AppExit, String> {
    // As Bevy's own runners do: plugins finish (the glTF loader is registered then).
    if app.plugins_state() != bevy::app::PluginsState::Cleaned {
        while app.plugins_state() == bevy::app::PluginsState::Adding {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();
    }
    let (tick_tx, tick_rx) = mpsc::sync_channel::<Eyes>(1);
    let (mesh_tx, mesh_rx) = mpsc::channel::<AssetMsg>();
    let slot = Arc::new(Mutex::new(None::<DrawList>));
    let quit = Arc::new(AtomicBool::new(false));
    let thread = {
        let (slot, quit) = (slot.clone(), quit.clone());
        std::thread::Builder::new()
            .name("glvr-render".into())
            .spawn(move || render_thread(tick_tx, mesh_rx, slot, quit))
            .map_err(|e| e.to_string())?
    };
    let mut extractor = Extractor::new(mesh_tx);
    let mut eyes = None;
    let mut last_log = Instant::now();
    let (mut updates, mut update_time, mut extract_time) = (0u32, Duration::ZERO, Duration::ZERO);
    let stop = |exit: AppExit| {
        quit.store(true, Ordering::Relaxed);
        thread.join().map_err(|_| "the render thread panicked".to_string())??;
        Ok(exit)
    };
    loop {
        // Paced by the headset's frames; idle (no session running) at 50 Hz.
        match tick_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(e) => {
                let mut input = app.world_mut().resource_mut::<XrInput>();
                input.hands = e.controllers;
                if let Some(head) = head_of(&e.views) {
                    input.head = Some(head);
                }
                eyes = Some(e);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return stop(AppExit::Success),
        }
        let t0 = Instant::now();
        app.update();
        if let Some(exit) = app.should_exit() {
            return stop(exit);
        }
        let t1 = Instant::now();
        let dirty = std::mem::take(&mut *app.world_mut().resource_mut::<DirtyMeshes>());
        extractor.forget(dirty.0.into_iter());
        let mut list = extractor.extract(app.world_mut(), eyes.as_ref());
        {
            let mut slot = slot.lock().map_err(|_| "the draw list's lock")?;
            // A list not drawn yet: its vibrations are kept.
            if let Some(old) = slot.as_ref() {
                for (h, o) in list.haptics.iter_mut().zip(old.haptics) {
                    *h = h.max(o);
                }
            }
            *slot = Some(list);
        }
        updates += 1;
        update_time += t1 - t0;
        extract_time += t1.elapsed();
        if last_log.elapsed() >= Duration::from_secs(5) {
            info!(
                "glvr: {:.1} updates/s, update {:.2} ms, extract {:.2} ms",
                updates as f32 / last_log.elapsed().as_secs_f32(),
                update_time.as_secs_f32() * 1000.0 / updates.max(1) as f32,
                extract_time.as_secs_f32() * 1000.0 / updates.max(1) as f32,
            );
            last_log = Instant::now();
            (updates, update_time, extract_time) = (0, Duration::ZERO, Duration::ZERO);
        }
    }
}

/// Ends `frame` showing the image drawn last (`shown`: its swapchain, eyes' poses and fence),
/// once the GPU is done with it; or nothing.
fn show(
    headset: &mut headset::Headset,
    renderer: &render::Renderer,
    frame: &headset::Frame,
    shown: Option<(usize, Vec<openxr::View>, Option<glow::Fence>)>,
) -> Result<(), String> {
    match shown {
        Some((chain, views, fence)) => {
            if let Some(fence) = fence {
                renderer.wait(fence);
            }
            headset.present(frame, chain, &views)
        }
        None => headset.end_empty(frame),
    }
}

/// The render thread: the OpenGL context, the session and its frames.
fn render_thread(
    tick: SyncSender<Eyes>,
    meshes: Receiver<AssetMsg>,
    slot: Arc<Mutex<Option<DrawList>>>,
    quit: Arc<AtomicBool>,
) -> Result<(), String> {
    let context = egl::Context::new()?;
    let mut renderer = render::Renderer::new(
        context.gl(),
        |name| context.egl.get_proc_address(name).map_or(std::ptr::null(), |f| f as *const std::ffi::c_void),
        samples(),
    )?;
    // Each frame's image is drawn with its own eyes and shown at once (waited for), into one
    // swapchain. LT_GL_DELAYED=1: shown a frame later instead, the next one drawn meanwhile
    // into a second swapchain (more time for the GPU, but SteamVR on the Steam Frame then shows
    // older images between new ones: a judder whenever the head turns).
    let direct = !std::env::var("LT_GL_DELAYED").is_ok_and(|v| v == "1");
    let mut headset = headset::Headset::new(&context, if direct { 1 } else { 2 })?;
    info!("glvr: eyes {}x{}, sRGB {}", headset.size.0, headset.size.1, headset.srgb);
    // LT_XR_SHOT=<secs>: both eyes' images saved that long into the run.
    let started = Instant::now();
    let mut shot = std::env::var("LT_XR_SHOT").ok().and_then(|v| v.parse::<f32>().ok());
    if !direct {
        info!("glvr: images shown a frame after they are drawn");
    }
    let mut list: Option<DrawList> = None;
    // The image drawn, its eyes' poses and the fence it is done at, to show.
    let mut drawn: Option<(usize, Vec<openxr::View>, Option<glow::Fence>)> = None;
    let mut last_log = Instant::now();
    let (mut frames, mut draw_time, mut draws, mut culled) = (0u32, Duration::ZERO, 0u64, 0u64);
    while !quit.load(Ordering::Relaxed) {
        if !headset.poll()? {
            return Ok(());
        }
        if !headset.running() {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let Some(frame) = headset.begin(!direct)? else { continue };
        // The main thread updates the game for the next frame meanwhile.
        let _ = tick.try_send(Eyes { views: frame.views.clone(), controllers: frame.controllers });
        let t0 = Instant::now();
        if let Some(new) = slot.lock().map_err(|_| "the draw list's lock")?.take() {
            for (i, &amount) in new.haptics.iter().enumerate() {
                if amount > 0.0 {
                    headset.pulse(i, amount);
                }
            }
            list = Some(new);
        }
        // (The meshes of the list taken were sent before it.)
        while let Ok(msg) = meshes.try_recv() {
            renderer.apply(msg);
        }
        // Delayed: the image drawn last frame is shown in this one, with the poses it was drawn
        // with, before this frame's is drawn.
        if !direct {
            show(&mut headset, &renderer, &frame, drawn.take())?;
        }
        let Some(list) = &list else {
            if direct {
                headset.end_empty(&frame)?;
            }
            continue;
        };
        let (chain, image) = headset.acquire()?;
        renderer.draw(list, &frame.views, image, headset.size, headset.srgb)?;
        if shot.is_some_and(|at| started.elapsed().as_secs_f32() >= at) {
            shot = None;
            if let Err(e) = renderer.save(image, headset.size) {
                warn!("glvr: {e}");
            }
        }
        let fence = renderer.fence();
        drawn = Some((chain, frame.views.clone(), fence));
        if direct {
            show(&mut headset, &renderer, &frame, drawn.take())?;
        }
        frames += 1;
        draw_time += t0.elapsed();
        draws += renderer.stats.draws as u64;
        culled += renderer.stats.culled as u64;
        if last_log.elapsed() >= Duration::from_secs(5) {
            let n = frames.max(1);
            let (lo, hi, ahead) = headset.pacing();
            info!(
                "glvr: {:.1} fps, draw {:.2} ms, {} draws ({} culled) a frame, runtime period {lo:.1}-{hi:.1} ms, eyes {ahead:.1} ms ahead",
                frames as f32 / last_log.elapsed().as_secs_f32(),
                draw_time.as_secs_f32() * 1000.0 / n as f32,
                draws / n as u64,
                culled / n as u64,
            );
            last_log = Instant::now();
            (frames, draw_time, draws, culled) = (0, Duration::ZERO, 0, 0);
        }
    }
    Ok(())
}
