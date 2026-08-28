//! Frame-time and per-frame allocation for the TUI renderer.
//!
//! This example drives the real `render` through a `ratatui` `TestBackend`, so
//! it needs no terminal. It measures three things and prints them as JSON, so
//! `bench/tui_frame.py` can parse a run.
//!
//! 1. Frame time under a streaming turn. It renders one frame per streamed
//!    delta and times each `terminal.draw`. It reports the 50th and the 99th
//!    percentile, and the sample count behind them.
//! 2. Allocation per frame in the steady state. A counting global allocator
//!    wraps the system allocator. The example renders a fixed transcript many
//!    times, with one `Terminal` reused, and divides the allocation count by the
//!    frame count. A reused terminal is the honest steady state, because a real
//!    loop draws into the same back buffer every frame.
//! 3. The motion sweep, measured on its own. The renderer applies the motion in
//!    `apply_sweep` by writing styles into cells that already exist, so it never
//!    allocates. It does not call `sweep_frame`, which returns a `Vec`. This section
//!    measures `sweep_frame` in isolation, as the allocating helper form, so a change
//!    to the sweep shows here as a micro-benchmark, separate from the whole-frame time.
//!
//! Every number is one process. `bench/tui_frame.py` runs the process three
//! times and takes the median, to match `bench/footprint.sh`.
//!
//! Run:
//!   cargo build --release -p rho-tui --example frame_bench
//!   target/release/examples/frame_bench            # JSON on stdout
//!   RHO_FRAMES=5000 target/release/examples/frame_bench

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_core::ToolKind;
use rho_tui::{ActivityState, MotionInputs, Row, ToolRowStatus, TuiState, render, sweep_frame};

/// A global allocator that counts every allocation while it is armed.
///
/// It counts a raw `alloc` and the growing half of a `realloc`, because both add
/// bytes the renderer holds for one frame. The counter is off until the steady
/// state, so process start and the warm-up frames do not pollute the number.
struct Counting;

static ARMED: AtomicU64 = AtomicU64::new(0);
static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) == 1 {
            ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) == 1 {
            ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            if new_size > layout.size() {
                ALLOC_BYTES.fetch_add((new_size - layout.size()) as u64, Ordering::Relaxed);
            }
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// The frame size. One hundred columns is the design reference width, and thirty
/// rows is a common terminal height. See `docs/design/tui-frames/100-idle.txt`.
const COLS: u16 = 100;
const ROWS: u16 = 30;

/// The inputs under which the sweep animates: motion on and a terminal stdout.
fn animating() -> MotionInputs {
    MotionInputs {
        tui_motion: true,
        stdout_is_terminal: true,
    }
}

/// Build a transcript that looks like the middle of a streaming turn: a spread of
/// assistant text, tool rows in each status, and a thinking row. The turn is
/// running, so the status word animates in the design's target renderer.
fn streaming_state() -> TuiState {
    let mut state = TuiState::default();
    state.model = "anthropic/claude-haiku-4.5".to_string();
    state.activity = ActivityState::Running;
    state.status = "working".to_string();
    for i in 0..40 {
        match i % 5 {
            0 => state.rows.push(Row::Tool {
                id: format!("call-{i}"),
                name: "bash".to_string(),
                kind: ToolKind::Execute,
                status: ToolRowStatus::Ok,
                preview: format!("ran step {i} and it printed a line of output"),
            }),
            1 => state.rows.push(Row::Thinking {
                text: format!("weighing option {i} against option {}", i + 1),
            }),
            _ => state.rows.push(Row::Assistant {
                text: format!("line {i}: the model streamed this sentence one delta at a time"),
            }),
        }
    }
    state
}

/// The percentile of a sorted slice, by the nearest-rank method.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn main() {
    let frames: usize = std::env::var("RHO_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000);

    let mut state = streaming_state();
    let backend = TestBackend::new(COLS, ROWS);
    let mut terminal = Terminal::new(backend).expect("build test terminal");

    // Warm up, so the first draw's one-time buffer growth does not skew the
    // timing or the allocation count.
    for _ in 0..64 {
        terminal
            .draw(|frame| render(&state, frame))
            .expect("warm-up draw");
    }

    // ---- Frame time under a streaming turn. -------------------------------
    // One delta per frame, so the transcript changes every draw, as it does in a
    // real streamed turn. Time each draw.
    let mut samples: Vec<f64> = Vec::with_capacity(frames);
    let mut tick: u64 = 0;
    let motion = animating();
    for _ in 0..frames {
        // Advance the streamed text by one delta, as a turn does.
        if let Some(Row::Assistant { text }) = state
            .rows
            .iter_mut()
            .rev()
            .find(|r| matches!(r, Row::Assistant { .. }))
        {
            text.push('.');
            if text.len() > 200 {
                text.truncate(60);
            }
        }
        let start = Instant::now();
        terminal
            .draw(|frame| render(&state, frame))
            .expect("timed draw");
        samples.push(start.elapsed().as_nanos() as f64 / 1000.0);
        tick = tick.wrapping_add(1);
    }
    let _ = tick;
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let render_p50 = percentile(&samples, 50.0);
    let render_p99 = percentile(&samples, 99.0);

    // ---- Allocation per frame in the steady state. ------------------------
    // A fixed transcript, one reused terminal, the counter armed. This is the
    // steady state: no row is added, so any allocation is the renderer's own
    // per-frame cost.
    let steady = streaming_state();
    for _ in 0..64 {
        terminal
            .draw(|frame| render(&steady, frame))
            .expect("steady warm-up");
    }
    let steady_frames = 2000u64;
    ALLOC_CALLS.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    ARMED.store(1, Ordering::Relaxed);
    for _ in 0..steady_frames {
        terminal
            .draw(|frame| render(&steady, frame))
            .expect("steady draw");
    }
    ARMED.store(0, Ordering::Relaxed);
    let alloc_calls = ALLOC_CALLS.load(Ordering::Relaxed);
    let alloc_bytes = ALLOC_BYTES.load(Ordering::Relaxed);
    let allocs_per_frame = alloc_calls as f64 / steady_frames as f64;
    let bytes_per_frame = alloc_bytes as f64 / steady_frames as f64;

    // ---- The motion sweep, on its own. ------------------------------------
    // The renderer applies the motion by writing cell styles directly, and does not
    // call `sweep_frame`. This measures `sweep_frame` in isolation, as the allocating
    // helper form, over the same steady frames.
    let mut motion_samples: Vec<f64> = Vec::with_capacity(frames);
    for t in 0..frames as u64 {
        let start = Instant::now();
        let cells = sweep_frame("working", t, &motion);
        motion_samples.push(start.elapsed().as_nanos() as f64 / 1000.0);
        std::hint::black_box(&cells);
    }
    motion_samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let motion_p50 = percentile(&motion_samples, 50.0);

    ALLOC_CALLS.store(0, Ordering::Relaxed);
    ARMED.store(1, Ordering::Relaxed);
    for t in 0..steady_frames {
        let cells = sweep_frame("working", t, &motion);
        std::hint::black_box(&cells);
    }
    ARMED.store(0, Ordering::Relaxed);
    let motion_allocs_per_call = ALLOC_CALLS.load(Ordering::Relaxed) as f64 / steady_frames as f64;

    println!(
        concat!(
            "{{\n",
            "  \"frame_size\": \"{cols}x{rows}\",\n",
            "  \"stream_samples\": {samples},\n",
            "  \"render_frame_us_p50\": {p50:.3},\n",
            "  \"render_frame_us_p99\": {p99:.3},\n",
            "  \"steady_frames\": {steady},\n",
            "  \"render_allocs_per_frame\": {apf:.3},\n",
            "  \"render_bytes_per_frame\": {bpf:.1},\n",
            "  \"motion_samples\": {msamples},\n",
            "  \"motion_sweep_us_p50\": {mp50:.4},\n",
            "  \"motion_allocs_per_call\": {mapc:.3},\n",
            "  \"motion_in_renderer\": true\n",
            "}}"
        ),
        cols = COLS,
        rows = ROWS,
        samples = samples.len(),
        p50 = render_p50,
        p99 = render_p99,
        steady = steady_frames,
        apf = allocs_per_frame,
        bpf = bytes_per_frame,
        msamples = motion_samples.len(),
        mp50 = motion_p50,
        mapc = motion_allocs_per_call,
    );
}
