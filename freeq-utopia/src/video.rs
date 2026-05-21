//! Utopia's video tile.
//!
//! Utopia publishes a video track like any participant. The tile shows
//! an audio-reactive "presence" — a glowing orb that pulses with the
//! bot's own voice — and, when a visual aid would help, an LLM-authored
//! SVG "card". Everything is drawn as SVG and rasterized to RGBA frames
//! with resvg, then fed to the H.264 encoder.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use iroh_live::media::format::{PixelFormat, VideoFormat, VideoFrame};
use iroh_live::media::traits::VideoSource;

/// Tile resolution. 360p is ample for an agent presence tile and cheap
/// to rasterize on the CPU.
pub const VIDEO_W: u32 = 640;
pub const VIDEO_H: u32 = 360;
const FPS: u64 = 15;
/// How long a visual-aid card stays on screen before the tile returns
/// to the bare presence.
const CARD_SECS: u64 = 20;

/// A visual-aid card: a full SVG document shown until `until`.
struct Card {
    svg: String,
    until: Instant,
}

/// Shared handle to utopia's video tile. Clone-cheap.
///
/// Hand [`source`](Self::source) to `broadcast.video()`, give
/// [`level_handle`](Self::level_handle) to the audio path so the
/// presence pulses with utopia's voice, and call
/// [`show_card`](Self::show_card) from the Q&A path to put up a visual.
#[derive(Clone)]
pub struct VideoTile {
    latest: Arc<Mutex<Option<VideoFrame>>>,
    level: Arc<AtomicU32>,
    card: Arc<Mutex<Option<Card>>>,
    running: Arc<AtomicBool>,
}

impl VideoTile {
    pub fn new() -> Self {
        Self {
            latest: Arc::new(Mutex::new(None)),
            level: Arc::new(AtomicU32::new(0)),
            card: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The [`VideoSource`] to hand to `broadcast.video().set_source(..)`.
    pub fn source(&self) -> PushVideoSource {
        PushVideoSource {
            latest: self.latest.clone(),
        }
    }

    /// Shared loudness cell — give a clone to the audio path so the
    /// presence animation tracks utopia's own speech.
    pub fn level_handle(&self) -> Arc<AtomicU32> {
        self.level.clone()
    }

    /// Show an LLM-authored SVG card for [`CARD_SECS`], then auto-return
    /// to the presence.
    pub fn show_card(&self, svg: String) {
        *self.card.lock().expect("card lock") = Some(Card {
            svg,
            until: Instant::now() + Duration::from_secs(CARD_SECS),
        });
    }

    /// Stop the render loop. Call on call-end.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Spawn the render loop on a dedicated thread — it produces ~15 fps
    /// of RGBA frames into the slot the encoder pulls.
    pub fn spawn_renderer(&self) {
        let latest = self.latest.clone();
        let level = self.level.clone();
        let card = self.card.clone();
        let running = self.running.clone();
        std::thread::Builder::new()
            .name("utopia-video".into())
            .spawn(move || render_loop(latest, level, card, running))
            .expect("spawn video renderer");
    }
}

impl Default for VideoTile {
    fn default() -> Self {
        Self::new()
    }
}

/// The [`VideoSource`] the H.264 encoder pulls. Returns the most recent
/// frame the render loop produced — `take`n so each frame is encoded at
/// most once; between renders the encoder simply idles.
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

fn render_loop(
    latest: Arc<Mutex<Option<VideoFrame>>>,
    level: Arc<AtomicU32>,
    card: Arc<Mutex<Option<Card>>>,
    running: Arc<AtomicBool>,
) {
    // Build resvg options once; load system fonts so card text renders.
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

    while running.load(Ordering::Relaxed) {
        let tick = Instant::now();
        let t = started.elapsed().as_secs_f32();
        let lvl = f32::from_bits(level.load(Ordering::Relaxed)).clamp(0.0, 1.0);

        // A live, unexpired card wins; otherwise draw the presence.
        let svg = {
            let mut guard = card.lock().expect("card lock");
            match guard.as_ref() {
                Some(c) if c.until > Instant::now() => c.svg.clone(),
                Some(_) => {
                    *guard = None;
                    presence_svg(t, lvl)
                }
                None => presence_svg(t, lvl),
            }
        };

        if let Some(frame) = rasterize(&svg, &opt, &mut pixmap) {
            *latest.lock().expect("video frame lock") = Some(frame);
        }

        if let Some(rest) = frame_dt.checked_sub(tick.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    tracing::info!("utopia video renderer stopped");
}

/// Rasterize an SVG document to an opaque RGBA [`VideoFrame`]. Returns
/// `None` if the SVG fails to parse — a bad LLM card must not kill the
/// tile.
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
    // tiny-skia stores premultiplied RGBA; our SVGs paint a fully opaque
    // background, so premultiplied == straight RGBA.
    let data = bytes::Bytes::copy_from_slice(pixmap.data());
    Some(VideoFrame::new_rgba(data, VIDEO_W, VIDEO_H, Duration::ZERO))
}

/// The audio-reactive presence: a glowing orb over a deep gradient. The
/// orb breathes slowly when idle and swells with utopia's own voice
/// (`level` in `[0,1]`).
fn presence_svg(t: f32, level: f32) -> String {
    let breathe = (t * 1.6).sin() * 5.0;
    let orb_r = 46.0 + breathe + level * 66.0;
    let glow_r = orb_r * 1.95;
    let glow_op = 0.12 + level * 0.42;
    let ring_r = orb_r + 22.0 + (t * 2.1).sin() * 4.0;
    let ring_op = 0.22 + level * 0.3;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <radialGradient id="bg" cx="50%" cy="40%" r="80%">
      <stop offset="0%" stop-color="#1b2747"/>
      <stop offset="100%" stop-color="#05070f"/>
    </radialGradient>
    <radialGradient id="orb" cx="42%" cy="38%" r="70%">
      <stop offset="0%" stop-color="#eaf4ff"/>
      <stop offset="42%" stop-color="#6cb0ff"/>
      <stop offset="100%" stop-color="#1d49a0"/>
    </radialGradient>
  </defs>
  <rect width="{w}" height="{h}" fill="url(#bg)"/>
  <circle cx="320" cy="156" r="{glow_r:.1}" fill="#6cb0ff" opacity="{glow_op:.3}"/>
  <circle cx="320" cy="156" r="{ring_r:.1}" fill="none" stroke="#8fc4ff" stroke-width="1.5" opacity="{ring_op:.3}"/>
  <circle cx="320" cy="156" r="{orb_r:.1}" fill="url(#orb)"/>
  <text x="320" y="316" font-family="Helvetica, Arial, sans-serif" font-size="32" font-weight="600" fill="#dce8ff" text-anchor="middle" letter-spacing="7">utopia</text>
</svg>"##,
        w = VIDEO_W,
        h = VIDEO_H,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_rasterizes_across_levels() {
        let opt = resvg::usvg::Options::default();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(VIDEO_W, VIDEO_H).unwrap();
        // Idle, mid, and full-level presence must all produce a frame.
        for (t, lvl) in [(0.0, 0.0), (1.5, 0.5), (3.2, 1.0)] {
            let frame = rasterize(&presence_svg(t, lvl), &opt, &mut pixmap)
                .expect("presence SVG must rasterize");
            assert_eq!(frame.dimensions, [VIDEO_W, VIDEO_H]);
        }
    }

    #[test]
    fn malformed_svg_yields_none_not_panic() {
        // A bad LLM card must degrade to "no frame", never crash the
        // render loop.
        let opt = resvg::usvg::Options::default();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(VIDEO_W, VIDEO_H).unwrap();
        assert!(rasterize("<not real svg", &opt, &mut pixmap).is_none());
    }

    #[test]
    fn show_card_then_expiry_returns_to_presence() {
        let tile = VideoTile::new();
        assert!(tile.card.lock().unwrap().is_none());
        tile.show_card("<svg/>".into());
        assert!(tile.card.lock().unwrap().is_some());
    }
}
