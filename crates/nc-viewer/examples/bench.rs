//! Synthetic toolpath benchmark for the viewer geometry layer.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p nc-viewer --example bench --release
//! ```
//!
//! The benchmark reports wall-clock time with `std::time::Instant` for path
//! construction, 2D render-plan generation, and 3D render-plan generation at
//! the two sizes in the performance budget: 200k and 1M segments.

use std::time::{Duration, Instant};

use nc_viewer::{Camera2D, Camera3D, RenderOptions, RenderPlan, Viewport, synthetic_toolpath};

fn main() {
    println!("nc-viewer synthetic toolpath benchmark");
    println!(
        "platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("viewport: 1920x1080, release build, 3 render-plan iterations");
    println!();

    for &count in &[200_000usize, 1_000_000] {
        benchmark_size(count);
    }
}

fn benchmark_size(count: usize) {
    let build_start = Instant::now();
    let toolpath = synthetic_toolpath(count);
    let build = build_start.elapsed();
    assert_eq!(toolpath.segment_count(), count);

    let bounds = toolpath.bounds();
    let camera2d = Camera2D::fitted(Viewport::new(1920.0, 1080.0), bounds.xy_bounds(), 24.0);
    let camera3d = Camera3D::fitted(Viewport::new(1920.0, 1080.0), bounds, 24.0);

    // Warm the CPU frequency and allocator, then measure three independent
    // plans. Each frame rebuilds the plan, so every iteration allocates.
    let mut plan2d = RenderPlan::from_2d(&toolpath, &camera2d, RenderOptions::default());
    let mut plan3d = RenderPlan::from_3d(&toolpath, &camera3d, RenderOptions::default());
    std::hint::black_box((plan2d.point_count(), plan3d.point_count()));

    let mut best2d = Duration::MAX;
    let mut total2d = Duration::ZERO;
    for _ in 0..3 {
        let start = Instant::now();
        plan2d = RenderPlan::from_2d(&toolpath, &camera2d, RenderOptions::default());
        let elapsed = start.elapsed();
        best2d = best2d.min(elapsed);
        total2d += elapsed;
    }

    let mut best3d = Duration::MAX;
    let mut total3d = Duration::ZERO;
    for _ in 0..3 {
        let start = Instant::now();
        plan3d = RenderPlan::from_3d(&toolpath, &camera3d, RenderOptions::default());
        let elapsed = start.elapsed();
        best3d = best3d.min(elapsed);
        total3d += elapsed;
    }

    std::hint::black_box((plan2d.point_count(), plan3d.point_count()));

    println!("segments: {count}");
    println!(
        "  build: {:>8.3} ms   memory: {:>7.1} MiB",
        millis(build),
        toolpath.memory_estimate() as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  2D:    best {:>8.3} ms   mean {:>8.3} ms   polylines {:>7}   points {:>8}",
        millis(best2d),
        millis(total2d / 3),
        plan2d.len(),
        plan2d.point_count()
    );
    println!(
        "  3D:    best {:>8.3} ms   mean {:>8.3} ms   polylines {:>7}   points {:>8}",
        millis(best3d),
        millis(total3d / 3),
        plan3d.len(),
        plan3d.point_count()
    );
    println!();
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
