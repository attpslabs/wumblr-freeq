//! Utopia's video tile.
//!
//! The tile is a live, animated surface — never a black square. When
//! utopia has nothing to show it renders a **state-aware presence** (an
//! orb that visibly idles, listens, thinks, or speaks). When it answers
//! a question it renders an **animated storyboard board**: a titled list
//! of points that draw themselves in one at a time, and that
//! *accumulates* across the call rather than flashing and vanishing.
//!
//! Everything is drawn as SVG, re-rendered every frame (so it genuinely
//! animates), rasterized with resvg, and fed to the H.264 encoder.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use iroh_live::media::format::{PixelFormat, VideoFormat, VideoFrame};
use iroh_live::media::traits::VideoSource;

/// Tile resolution. 360p is ample and cheap to rasterize on the CPU.
pub const VIDEO_W: u32 = 640;
pub const VIDEO_H: u32 = 360;
const FPS: u64 = 15;
/// Most steps a board shows at once (oldest drop off beyond this).
const MAX_STEPS: usize = 6;

/// What utopia is doing — read off the audio + a "thinking" flag and
/// shown by the presence.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mood {
    Idle,
    Listening,
    Thinking,
    Speaking,
}

/// An animated storyboard board: a title plus a list of points. Points
/// shared as a prefix with the previous board are "carried" — they snap
/// in instantly; only genuinely new points animate, so the board
/// accumulates across the call instead of redrawing.
struct Scene {
    title: String,
    steps: Vec<String>,
    carried: usize,
    shown_at: Instant,
}

/// Shared handle to utopia's video tile. Clone-cheap.
#[derive(Clone)]
pub struct VideoTile {
    latest: Arc<Mutex<Option<VideoFrame>>>,
    /// utopia's own speech loudness, `f32` bits in `[0,1]`.
    level: Arc<AtomicU32>,
    /// Loudest participant's loudness — drives the "listening" mood.
    peer_level: Arc<AtomicU32>,
    /// Set while an LLM call is in flight — drives the "thinking" mood.
    thinking: Arc<AtomicBool>,
    scene: Arc<Mutex<Option<Scene>>>,
    running: Arc<AtomicBool>,
}

impl VideoTile {
    pub fn new() -> Self {
        Self {
            latest: Arc::new(Mutex::new(None)),
            level: Arc::new(AtomicU32::new(0)),
            peer_level: Arc::new(AtomicU32::new(0)),
            thinking: Arc::new(AtomicBool::new(false)),
            scene: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The [`VideoSource`] to hand to `broadcast.video().set_source(..)`.
    pub fn source(&self) -> PushVideoSource {
        PushVideoSource {
            latest: self.latest.clone(),
        }
    }

    /// Loudness cell for utopia's own voice (the audio path writes it).
    pub fn level_handle(&self) -> Arc<AtomicU32> {
        self.level.clone()
    }

    /// Loudness cell for incoming participant audio (a tap writes it).
    pub fn peer_level_handle(&self) -> Arc<AtomicU32> {
        self.peer_level.clone()
    }

    /// Mark whether an LLM call is in flight (drives the thinking mood).
    pub fn set_thinking(&self, on: bool) {
        self.thinking.store(on, Ordering::Relaxed);
    }

    /// The board's current points — pass these to the model so the next
    /// answer can carry the still-relevant ones forward.
    pub fn board_steps(&self) -> Vec<String> {
        self.scene
            .lock()
            .expect("scene lock")
            .as_ref()
            .map(|s| s.steps.clone())
            .unwrap_or_default()
    }

    /// Replace the board. Points that match the current board as a
    /// leading prefix are "carried" (snap in); the rest animate.
    pub fn show_scene(&self, title: String, mut steps: Vec<String>) {
        steps.truncate(MAX_STEPS);
        let mut guard = self.scene.lock().expect("scene lock");
        let carried = match guard.as_ref() {
            Some(prev) => prev
                .steps
                .iter()
                .zip(&steps)
                .take_while(|(a, b)| a == b)
                .count(),
            None => 0,
        };
        *guard = Some(Scene {
            title,
            steps,
            carried,
            shown_at: Instant::now(),
        });
    }

    /// Stop the render loop. Call on call-end.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Spawn the render loop on a dedicated thread.
    pub fn spawn_renderer(&self) {
        let tile = self.clone();
        std::thread::Builder::new()
            .name("utopia-video".into())
            .spawn(move || tile.render_loop())
            .expect("spawn video renderer");
    }

    fn render_loop(self) {
        let mut opt = resvg::usvg::Options::default();
        opt.fontdb_mut().load_system_fonts();
        let mut pixmap = match resvg::tiny_skia::Pixmap::new(VIDEO_W, VIDEO_H) {
            Some(p) => p,
            None => {
                tracing::error!("video: could not allocate pixmap");
                return;
            }
        };
        let frame_dt = Duration::from_millis(1000 / FPS);
        let started = Instant::now();
        tracing::info!("utopia video renderer started ({VIDEO_W}x{VIDEO_H} @ {FPS}fps)");

        while self.running.load(Ordering::Relaxed) {
            let tick = Instant::now();
            let t = started.elapsed().as_secs_f32();
            let level = f32::from_bits(self.level.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let peer = f32::from_bits(self.peer_level.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let thinking = self.thinking.load(Ordering::Relaxed);
            let mood = if level > 0.03 {
                Mood::Speaking
            } else if thinking {
                Mood::Thinking
            } else if peer > 0.03 {
                Mood::Listening
            } else {
                Mood::Idle
            };

            let svg = {
                let guard = self.scene.lock().expect("scene lock");
                match guard.as_ref() {
                    Some(scene) => scene_svg(scene, level, mood),
                    None => presence_svg(mood, t, level, peer),
                }
            };

            if let Some(frame) = rasterize(&svg, &opt, &mut pixmap) {
                *self.latest.lock().expect("video frame lock") = Some(frame);
            }

            if let Some(rest) = frame_dt.checked_sub(tick.elapsed()) {
                std::thread::sleep(rest);
            }
        }
        tracing::info!("utopia video renderer stopped");
    }
}

impl Default for VideoTile {
    fn default() -> Self {
        Self::new()
    }
}

/// The [`VideoSource`] the H.264 encoder pulls — the most recent
/// rendered frame, `take`n so each frame is encoded at most once.
pub struct PushVideoSource {
    latest: Arc<Mutex<Option<VideoFrame>>>,
}

impl VideoSource for PushVideoSource {
    fn name(&self) -> &str {
        "utopia"
    }
    fn format(&self) -> VideoFormat {
        VideoFormat {
            pixel_format: PixelFormat::Rgba,
            dimensions: [VIDEO_W, VIDEO_H],
        }
    }
    fn pop_frame(&mut self) -> anyhow::Result<Option<VideoFrame>> {
        Ok(self.latest.lock().expect("video frame lock").take())
    }
    fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Cubic ease-out — fast in, gentle settle.
fn ease_out(p: f32) -> f32 {
    let q = 1.0 - p.clamp(0.0, 1.0);
    1.0 - q * q * q
}

/// Rasterize an SVG document to an opaque RGBA [`VideoFrame`]. Returns
/// `None` if the SVG fails to parse — a bad scene must not kill the tile.
fn rasterize(
    svg: &str,
    opt: &resvg::usvg::Options,
    pixmap: &mut resvg::tiny_skia::Pixmap,
) -> Option<VideoFrame> {
    let tree = match resvg::usvg::Tree::from_str(svg, opt) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "video: SVG parse failed");
            return None;
        }
    };
    pixmap.fill(resvg::tiny_skia::Color::BLACK);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let data = bytes::Bytes::copy_from_slice(pixmap.data());
    Some(VideoFrame::new_rgba(data, VIDEO_W, VIDEO_H, Duration::ZERO))
}

/// Per-mood accent colour for the presence + the corner status dot.
fn mood_color(mood: Mood) -> &'static str {
    match mood {
        Mood::Idle => "#6cb0ff",
        Mood::Listening => "#54e2c8",
        Mood::Thinking => "#b594ff",
        Mood::Speaking => "#9fd2ff",
    }
}

/// The state-aware presence: a glowing orb whose colour and motion say
/// what utopia is doing — idle, listening, thinking, or speaking.
fn presence_svg(mood: Mood, t: f32, level: f32, peer: f32) -> String {
    let accent = mood_color(mood);
    let breathe = (t * 1.6).sin() * 5.0;
    let orb_r = match mood {
        Mood::Speaking => 48.0 + breathe + level * 64.0,
        Mood::Thinking => 44.0 + (t * 4.0).sin() * 4.0,
        Mood::Listening => 46.0 + breathe + peer * 30.0,
        Mood::Idle => 44.0 + breathe,
    };
    let glow_r = orb_r * 1.95;
    let glow_op = match mood {
        Mood::Speaking => 0.14 + level * 0.4,
        Mood::Thinking => 0.18 + (t * 4.0).sin().abs() * 0.12,
        Mood::Listening => 0.16 + peer * 0.3,
        Mood::Idle => 0.12,
    };

    // Mood-specific overlay: a rotating dashed ring while thinking,
    // contracting ripples while listening, a steady ring otherwise.
    let overlay = match mood {
        Mood::Thinking => format!(
            r##"<circle cx="320" cy="156" r="{r:.1}" fill="none" stroke="{accent}" stroke-width="3" stroke-dasharray="14 12" opacity="0.8" transform="rotate({deg:.1} 320 156)"/>"##,
            r = orb_r + 26.0,
            deg = t * 150.0,
        ),
        Mood::Listening => {
            let mut rings = String::new();
            for i in 0..3 {
                let phase = (t * 0.6 + i as f32 * 0.33).fract();
                let rr = orb_r + 8.0 + phase * 64.0;
                let op = (1.0 - phase) * 0.5;
                rings.push_str(&format!(
                    r##"<circle cx="320" cy="156" r="{rr:.1}" fill="none" stroke="{accent}" stroke-width="2" opacity="{op:.3}"/>"##,
                ));
            }
            rings
        }
        _ => format!(
            r##"<circle cx="320" cy="156" r="{r:.1}" fill="none" stroke="{accent}" stroke-width="1.5" opacity="0.3"/>"##,
            r = orb_r + 22.0 + (t * 2.0).sin() * 3.0,
        ),
    };

    let label = match mood {
        Mood::Idle => "utopia",
        Mood::Listening => "listening",
        Mood::Thinking => "thinking",
        Mood::Speaking => "utopia",
    };

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <radialGradient id="bg" cx="50%" cy="40%" r="80%">
      <stop offset="0%" stop-color="#16213f"/>
      <stop offset="100%" stop-color="#05070f"/>
    </radialGradient>
    <radialGradient id="orb" cx="42%" cy="38%" r="70%">
      <stop offset="0%" stop-color="#f2f7ff"/>
      <stop offset="44%" stop-color="{accent}"/>
      <stop offset="100%" stop-color="#16306a"/>
    </radialGradient>
  </defs>
  <rect width="{w}" height="{h}" fill="url(#bg)"/>
  <circle cx="320" cy="156" r="{glow_r:.1}" fill="{accent}" opacity="{glow_op:.3}"/>
  {overlay}
  <circle cx="320" cy="156" r="{orb_r:.1}" fill="url(#orb)"/>
  <text x="320" y="318" font-family="Helvetica, Arial, sans-serif" font-size="26" font-weight="600" fill="#cfe0ff" text-anchor="middle" letter-spacing="6">{label}</text>
</svg>"##,
        w = VIDEO_W,
        h = VIDEO_H,
    )
}

/// The animated storyboard board: a title and a column of point pills
/// that reveal in sequence (carried-over points snap in, new ones slide
/// + fade in with their connector drawing). A small corner dot keeps
/// utopia's live mood visible.
fn scene_svg(scene: &Scene, level: f32, mood: Mood) -> String {
    let elapsed = scene.shown_at.elapsed().as_secs_f32();
    let accent = mood_color(mood);
    let title_p = ease_out(elapsed / 0.4);

    let pill_x = 60.0_f32;
    let pill_w = 520.0_f32;
    let pill_h = 40.0_f32;
    let row_h = 50.0_f32;
    let first_y = 96.0_f32;

    let mut steps = String::new();
    for (i, text) in scene.steps.iter().enumerate() {
        let reveal = if i < scene.carried {
            0.0
        } else {
            0.3 + (i - scene.carried) as f32 * 0.6
        };
        let p = ease_out((elapsed - reveal) / 0.5);
        if p <= 0.0 {
            continue;
        }
        let y = first_y + i as f32 * row_h;
        let dy = (1.0 - p) * 16.0;
        let cy = y + pill_h / 2.0;
        let esc = xml_escape(text);
        // Connector from the previous pill, drawing in as the step
        // reveals (dash offset shrinks to zero).
        let connector = if i > 0 {
            let len = row_h - pill_h;
            format!(
                r##"<line x1="{cx:.1}" y1="{y0:.1}" x2="{cx:.1}" y2="{y:.1}" stroke="{accent}" stroke-width="2" stroke-dasharray="{len:.1}" stroke-dashoffset="{off:.1}" opacity="0.7"/>"##,
                cx = pill_x + 22.0,
                y0 = y - (row_h - pill_h),
                off = len * (1.0 - p),
            )
        } else {
            String::new()
        };
        steps.push_str(&format!(
            r##"{connector}<g opacity="{p:.3}" transform="translate(0 {dy:.1})">
    <rect x="{pill_x:.1}" y="{y:.1}" width="{pill_w:.1}" height="{pill_h:.1}" rx="11" fill="#16244c" stroke="#33508f" stroke-width="1"/>
    <circle cx="{dotx:.1}" cy="{cy:.1}" r="7" fill="{accent}"/>
    <text x="{tx:.1}" y="{ty:.1}" font-family="Helvetica, Arial, sans-serif" font-size="18" fill="#dde9ff">{esc}</text>
  </g>
"##,
            dotx = pill_x + 22.0,
            tx = pill_x + 44.0,
            ty = cy + 6.0,
        ));
    }

    let dot_r = 6.0 + level * 7.0;
    let title = xml_escape(&scene.title);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#0e1530"/>
      <stop offset="100%" stop-color="#070a16"/>
    </linearGradient>
  </defs>
  <rect width="{w}" height="{h}" fill="url(#bg)"/>
  <g opacity="{title_p:.3}">
    <text x="40" y="54" font-family="Helvetica, Arial, sans-serif" font-size="27" font-weight="700" fill="#eaf2ff">{title}</text>
    <line x1="40" y1="68" x2="600" y2="68" stroke="#2a3c70" stroke-width="1.5"/>
  </g>
  {steps}
  <circle cx="606" cy="332" r="{dot_r:.1}" fill="{accent}"/>
</svg>"##,
        w = VIDEO_W,
        h = VIDEO_H,
    )
}

/// Escape the five XML metacharacters so model-authored text can't
/// break the SVG document.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt() -> resvg::usvg::Options<'static> {
        resvg::usvg::Options::default()
    }

    #[test]
    fn presence_rasterizes_in_every_mood() {
        let opt = opt();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(VIDEO_W, VIDEO_H).unwrap();
        for mood in [Mood::Idle, Mood::Listening, Mood::Thinking, Mood::Speaking] {
            let frame = rasterize(&presence_svg(mood, 1.7, 0.5, 0.4), &opt, &mut pixmap)
                .expect("presence must rasterize");
            assert_eq!(frame.dimensions, [VIDEO_W, VIDEO_H]);
        }
    }

    #[test]
    fn scene_rasterizes_while_animating() {
        let opt = opt();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(VIDEO_W, VIDEO_H).unwrap();
        let scene = Scene {
            title: "Demo plan".into(),
            steps: vec!["Build it".into(), "Ship it".into(), "Profit".into()],
            carried: 1,
            shown_at: Instant::now(),
        };
        // Partway through the reveal, and fully revealed.
        for _ in 0..3 {
            let frame = rasterize(&scene_svg(&scene, 0.3, Mood::Speaking), &opt, &mut pixmap)
                .expect("scene must rasterize");
            assert_eq!(frame.dimensions, [VIDEO_W, VIDEO_H]);
        }
    }

    #[test]
    fn xml_escape_neutralizes_markup() {
        assert_eq!(xml_escape("a<b>&\"c"), "a&lt;b&gt;&amp;&quot;c");
    }

    #[test]
    fn show_scene_carries_a_common_prefix() {
        let tile = VideoTile::new();
        tile.show_scene("T".into(), vec!["a".into(), "b".into()]);
        // Same prefix + a new point → 2 carried, 1 fresh.
        tile.show_scene(
            "T".into(),
            vec!["a".into(), "b".into(), "c".into()],
        );
        let guard = tile.scene.lock().unwrap();
        let s = guard.as_ref().unwrap();
        assert_eq!(s.carried, 2);
        assert_eq!(s.steps.len(), 3);
    }

    #[test]
    fn show_scene_caps_step_count() {
        let tile = VideoTile::new();
        let many: Vec<String> = (0..20).map(|i| i.to_string()).collect();
        tile.show_scene("T".into(), many);
        assert_eq!(tile.board_steps().len(), MAX_STEPS);
    }
}
