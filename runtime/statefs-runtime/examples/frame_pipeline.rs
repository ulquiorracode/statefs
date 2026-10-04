//! # Host_Frame U-Cycle Pipeline Example (stitch-rs + StateFS)
//!
//! Demonstrates:
//! - Full integration of stitch-rs monomorphic U-cycle pipeline with StateFS.
//! - Middleware Layer 1 (Descent/Ascent): `FrameBoundaryMiddleware` enforcing delta-time clamping & invariant sanity.
//! - Middleware Layer 2 (Descent/Ascent): `FrameTelemetryMiddleware` recording execution latency and throughput.
//! - Terminal Handler: `HostFrameTerminal` executing the frame against StateFS using pre-cached zero-alloc `PathHandle`s.
//! - Complete zero-allocation execution inside a high-frequency (128Hz) engine frame loop.

use statefs_core::path::PathHandle;
use statefs_core::{MemStore, Path, Store, Value};
use std::time::Instant;
use stitch_rs::flow::FlowControl;
use stitch_rs::middleware::{Middleware, TerminalHandler};
use stitch_rs::pipeline::Pipeline;

/// Per-frame contextual state passed down and up the U-cycle pipeline.
#[derive(Debug, Default)]
pub struct HostFrameContext {
    pub frame_index: u64,
    pub total_dispatches: u64,
    pub clamped_frames: u64,
    pub total_simulated_ticks: u64,
    pub total_nanos: u128,
}

/// Incoming intent for a single engine frame tick (`Host_Frame`).
#[derive(Debug, Clone, Copy)]
pub struct HostFrameIntent {
    pub delta_time: f32,
    pub request_cheats: bool,
}

/// Atomic outcome produced at the terminal and modified during ascent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostFrameOutcome {
    pub tickrate: i64,
    pub gravity: f64,
    pub executed_substeps: u32,
    pub cheats_active: bool,
}

/// Potential pipeline errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFrameError {
    InvalidDeltaTime,
    SecurityViolation,
}

/// Middleware Layer 1: Boundary & Sanity Validation (Descent) and Invariant Check (Ascent).
pub struct FrameBoundaryMiddleware;

impl Middleware<HostFrameContext, HostFrameIntent, HostFrameOutcome, HostFrameError>
    for FrameBoundaryMiddleware
{
    fn on_enter(
        &self,
        ctx: &mut HostFrameContext,
        mut intent: HostFrameIntent,
    ) -> FlowControl<HostFrameIntent, HostFrameOutcome, HostFrameError> {
        // Guard against negative or NaN delta-times (crash/exploit attempt)
        if intent.delta_time <= 0.0 || intent.delta_time.is_nan() {
            return FlowControl::Halt(HostFrameError::InvalidDeltaTime);
        }

        // Clamp maximum delta-time to 100ms to avoid physics explosion (spiral of death)
        if intent.delta_time > 0.10 {
            ctx.clamped_frames = ctx.clamped_frames.saturating_add(1);
            intent.delta_time = 0.10;
        }

        FlowControl::Proceed(intent)
    }

    fn on_exit(
        &self,
        ctx: &mut HostFrameContext,
        outcome: &mut Result<HostFrameOutcome, HostFrameError>,
    ) {
        if let Ok(frame) = outcome {
            ctx.total_simulated_ticks = ctx
                .total_simulated_ticks
                .saturating_add(frame.executed_substeps as u64);
        }
    }
}

/// Middleware Layer 2: Telemetry & Monomorphic Latency Profiler.
pub struct FrameTelemetryMiddleware {
    start_time: std::cell::Cell<Option<Instant>>,
}

impl FrameTelemetryMiddleware {
    pub fn new() -> Self {
        Self {
            start_time: std::cell::Cell::new(None),
        }
    }
}

impl Default for FrameTelemetryMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl Middleware<HostFrameContext, HostFrameIntent, HostFrameOutcome, HostFrameError>
    for FrameTelemetryMiddleware
{
    fn on_enter(
        &self,
        ctx: &mut HostFrameContext,
        intent: HostFrameIntent,
    ) -> FlowControl<HostFrameIntent, HostFrameOutcome, HostFrameError> {
        ctx.frame_index = ctx.frame_index.saturating_add(1);
        ctx.total_dispatches = ctx.total_dispatches.saturating_add(1);
        self.start_time.set(Some(Instant::now()));
        FlowControl::Proceed(intent)
    }

    fn on_exit(
        &self,
        ctx: &mut HostFrameContext,
        _outcome: &mut Result<HostFrameOutcome, HostFrameError>,
    ) {
        if let Some(start) = self.start_time.get() {
            let elapsed = start.elapsed().as_nanos();
            ctx.total_nanos = ctx.total_nanos.saturating_add(elapsed);
        }
    }
}

/// Terminal Handler (The Point of Puncture / Дно буквы U).
/// Reads state via pre-cached O(1) PathHandles directly from StateFS MemStore.
pub struct HostFrameTerminal {
    store: MemStore,
    h_tickrate: PathHandle,
    h_gravity: PathHandle,
    h_cheats: PathHandle,
}

impl HostFrameTerminal {
    /// Pre-resolves direct O(1) [`PathHandle`]s for hot-loop execution.
    ///
    /// # Invariants & Preconditions
    /// Pinned handles remain valid as long as target CVAR nodes are not removed or cleared
    /// from the store. Mutations that update existing values preserve node indices, while
    /// structural deletions (`remove`/`clear`) require re-resolving handles.
    pub fn new(store: MemStore) -> Self {
        let h_tickrate = store
            .resolve_handle("/server/net/tickrate")
            .expect("tickrate handle");
        let h_gravity = store
            .resolve_handle("/server/physics/gravity")
            .expect("gravity handle");
        let h_cheats = store
            .resolve_handle("/server/security/allow_cheats")
            .expect("cheats handle");

        Self {
            store,
            h_tickrate,
            h_gravity,
            h_cheats,
        }
    }

    pub fn store_mut(&mut self) -> &mut MemStore {
        &mut self.store
    }
}

impl TerminalHandler<HostFrameContext, HostFrameIntent, HostFrameOutcome, HostFrameError>
    for HostFrameTerminal
{
    fn execute(
        &mut self,
        _ctx: &mut HostFrameContext,
        intent: HostFrameIntent,
    ) -> Result<HostFrameOutcome, HostFrameError> {
        // Zero-alloc, O(1) direct arena node lookups via PathHandles (2-3 ns)
        let tickrate = self
            .store
            .get_by_handle(self.h_tickrate)
            .and_then(|n| n.value.as_int())
            .unwrap_or(64);

        let gravity = self
            .store
            .get_by_handle(self.h_gravity)
            .and_then(|n| n.value.as_float())
            .unwrap_or(800.0);

        let allow_cheats = self
            .store
            .get_by_handle(self.h_cheats)
            .and_then(|n| n.value.as_bool())
            .unwrap_or(false);

        // Security check: Reject unauthorized cheat commands
        if intent.request_cheats && !allow_cheats {
            return Err(HostFrameError::SecurityViolation);
        }

        // Compute simulated physics substeps
        let substeps = ((intent.delta_time * (tickrate as f32)).round() as u32).max(1);

        Ok(HostFrameOutcome {
            tickrate,
            gravity,
            executed_substeps: substeps,
            cheats_active: intent.request_cheats && allow_cheats,
        })
    }
}

fn main() {
    println!("=== stitch-rs Monomorphic U-Cycle Frame Pipeline Example ===");

    // 1. Initialize StateFS engine store
    let mut store = MemStore::new();
    store
        .insert(&Path::parse("/server/net/tickrate"), Value::from(128))
        .expect("tickrate");
    store
        .insert(&Path::parse("/server/physics/gravity"), Value::from(800.0))
        .expect("gravity");
    store
        .insert(
            &Path::parse("/server/security/allow_cheats"),
            Value::from(false),
        )
        .expect("allow_cheats");

    println!("Initialized StateFS store with server cvars.");

    // 2. Construct monomorphic stitch-rs Pipeline with 2 layers of middleware
    // Pipeline: FrameBoundaryMiddleware -> FrameTelemetryMiddleware -> HostFrameTerminal
    let terminal = HostFrameTerminal::new(store);
    let mut pipeline = Pipeline::on_terminal(terminal)
        .use_middleware(FrameTelemetryMiddleware::new())
        .use_middleware(FrameBoundaryMiddleware);

    let mut ctx = HostFrameContext::default();

    // 3. Simulate 64 server ticks at 128Hz (dt = 1/128 ≈ 0.0078125s)
    let dt = 1.0 / 128.0;
    println!("\nExecuting 64 frames through monomorphic stitch-rs U-cycle...");

    for _frame in 1..=64 {
        let intent = HostFrameIntent {
            delta_time: dt,
            request_cheats: false,
        };

        let outcome = pipeline
            .dispatch(&mut ctx, intent)
            .expect("frame dispatch must succeed");

        assert_eq!(outcome.tickrate, 128);
        assert_eq!(outcome.gravity, 800.0);
    }

    // 4. Test security middleware rejection (attempting cheat request when cheats are disabled)
    let cheat_intent = HostFrameIntent {
        delta_time: dt,
        request_cheats: true,
    };
    let cheat_result = pipeline.dispatch(&mut ctx, cheat_intent);
    assert_eq!(cheat_result, Err(HostFrameError::SecurityViolation));
    println!("Security verification: Unauthorized cheat request blocked by terminal barrier.");

    // 5. Test boundary clamping (oversized dt > 0.1s clamped by FrameBoundaryMiddleware)
    let lag_intent = HostFrameIntent {
        delta_time: 0.50, // Massive lag spike
        request_cheats: false,
    };
    let lag_outcome = pipeline
        .dispatch(&mut ctx, lag_intent)
        .expect("lag frame clamped");
    assert!(ctx.clamped_frames > 0);
    println!(
        "Lag compensation verification: Delta time 0.5s clamped to 0.1s (simulated substeps: {}).",
        lag_outcome.executed_substeps
    );

    // 6. Test invalid dt rejection (< 0.0 rejected at descent)
    let negative_intent = HostFrameIntent {
        delta_time: -0.01,
        request_cheats: false,
    };
    let neg_result = pipeline.dispatch(&mut ctx, negative_intent);
    assert_eq!(neg_result, Err(HostFrameError::InvalidDeltaTime));
    println!("Descent guard verification: Negative delta-time halted in FrameBoundaryMiddleware.");

    // 7. Print context telemetry recorded by FrameTelemetryMiddleware
    let avg_latency = if ctx.total_dispatches > 0 {
        (ctx.total_nanos as f64) / (ctx.total_dispatches as f64)
    } else {
        0.0
    };

    println!("\n=== Pipeline Execution Telemetry ===");
    println!("Total frame dispatches: {}", ctx.total_dispatches);
    println!("Total simulated ticks:  {}", ctx.total_simulated_ticks);
    println!("Clamped lag spikes:     {}", ctx.clamped_frames);
    println!(
        "Average frame cycle latency (includes Instant timer): {:.2} ns / dispatch",
        avg_latency
    );
    println!(
        "StateFS + stitch-rs integration verified (zero per-frame path lookups/allocations in terminal path)."
    );
}
