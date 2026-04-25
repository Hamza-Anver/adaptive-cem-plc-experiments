//! Cross-Entropy Method (CEM) RL Fuzzer for PLC targets.
//!
//! # How it works
//!
//! The CEM maintains one categorical probability distribution per FSM phase
//! (IDLE / PRIME / FLOW / FILL), one per input byte position (7 for
//! pipeline_deep_oob).  Entry `dists[phase][byte_pos][byte_val]` is the
//! probability of choosing that byte when the PLC is (approximately) in that
//! phase.
//!
//! Every POPULATION_SIZE executions ("one generation"):
//!   1. Sort episode buffer by total reward (descending).
//!   2. Take the top ELITE_COUNT episodes as the "elite set".
//!   3. For every (phase, byte_pos), count byte-value frequencies in elite ticks.
//!   4. EMA-blend current distribution toward that frequency.
//!   5. Re-normalise; enforce MIN_PROB floor so every byte stays explorable.
//!
//! ## Reward per tick
//!   -0.001  time penalty (encourages short solutions)
//!   +15.0   each phase transition (IDLE→PRIME, PRIME→FLOW, FLOW→FILL)
//!   +2.0    per unit prime_score increase    (guides PRIME-phase learning)
//!   +2.0    per unit flux_score increase     (guides FLOW-phase learning)
//!   +0.5    per unit accumulator increase    (guides FILL-threshold learning)
//!   +3.0    per unit fill_head advance       (direct progress signal)
//!   +10.0   novelty bonus (fill_head value never seen before)
//!
//! ## Mutator strategy
//! - 60 % of the time: **extend** a corpus entry (keep N ticks as prefix,
//!   sample the remainder from CEM distributions).  This preserves hard-won
//!   PRIME/FLOW progress and lets the CEM focus on learning FILL.
//! - 40 % of the time: **fresh episode** sampled entirely from scratch.
//!
//! Phase attribution in the harness always uses the ACTUAL PLC phase from
//! state, not the heuristic — so distribution updates are always accurate.

use std::borrow::Cow;
use std::collections::HashSet;
use std::mem::size_of;

use libafl::{
    corpus::{InMemoryCorpus, OnDiskCorpus},
    events::SimpleEventManager,
    executors::inprocess::InProcessExecutor,
    feedback_or,
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Evaluator, Fuzzer, StdFuzzer},
    inputs::{BytesInput, HasTargetBytes},
    monitors::SimpleMonitor,
    mutators::{MutationResult, Mutator},
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasRand, StdState},
};
use libafl_bolts::{AsSlice, Named, rands::Rand, rands::StdRand, tuples::tuple_list};
use libafl_targets::std_edges_map_observer;
use libafl::observers::StdMapObserver;

use crate::common::{
    harness_boot_plc, harness_reset_plc,
    plc_get_full_state, plc_get_input_size, plc_step,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Bytes per PLC scan cycle (must match the target's INPUT_SIZE).
const TICK_SIZE: usize = 7;

/// Number of distributions tracked.
/// Slots 0-3 = FSM phases (IDLE/PRIME/FLOW/FILL-generic).
/// Slots 4-11 = FILL zone sub-distributions (zone 0 = fill_head 0-7,
///              zone 1 = 8-15, …, zone 7 = 56-63).
/// The mutator samples from the FILL-generic slot (3) when it doesn't
/// know the zone; the harness attributes ticks to zone-specific slots
/// so the CEM learns zone-differentiated byte values.
const NUM_PHASES: usize = 12;
const FILL_ZONE_BASE: usize = 4; // first zone-specific distribution index

/// Episodes collected before each CEM distribution update.
const POPULATION_SIZE: usize = 64;

/// Elite episodes used to update distributions (~20 % of POPULATION_SIZE).
const ELITE_COUNT: usize = 13;

/// EMA weight toward elite frequency each generation (0 < lr ≤ 1).
const LEARNING_RATE: f32 = 0.15;

/// Minimum per-byte probability floor – keeps all bytes explorable.
const MIN_PROB: f32 = 0.001;

/// Maximum ticks in a single sampled episode.
const MAX_TICKS: usize = 600;

/// Minimum ticks in a sampled episode (enough to cover all three transitions
/// at their typical timing, plus a stretch of FILL phase).
const MIN_TICKS: usize = 120;

/// LibAFL metrics slots: [phase, prime_score_u8, flux_score_u8, max_fill_head_u8]
const METRICS_SIZE: usize = 4;
static mut CEM_METRICS: [u8; METRICS_SIZE] = [0u8; METRICS_SIZE];

// ─── PLC state mirror ─────────────────────────────────────────────────────────
//
// Must match the C layout of PipelineState in pipeline_deep_oob.c exactly.

#[repr(C)]
struct PipelineState {
    phase:         u32,
    cycle_count:   i32,
    prime_cycles:  i32,
    prime_score:   i32,
    flux_score:    i32,
    flow_accum:    i32,
    press_accum:   i32,
    temp_accum:    i32,
    phase_counter: i32,
    fill_head:     i32,
    buffer:        [i32; 64],
    pv_sum:        i32,
    status:        i8,
}

impl Default for PipelineState {
    fn default() -> Self {
        // SAFETY: all fields are plain integers; zero-init is valid.
        unsafe { std::mem::zeroed() }
    }
}

// ─── Phase-time heuristic ─────────────────────────────────────────────────────
//
// Used by the mutator to decide *which* phase distribution to sample from
// when generating fresh episode bytes at position `t`.
//
// Pipeline timing (from pipeline_smoke.rs):
//   tick 0         : IDLE  (send cmd=1)
//   ticks  1 – 25  : PRIME (~8 checks × 3-cycle cadence → prime_score=8)
//   ticks 26 – 65  : FLOW  (40 ticks; flux_score halved every 11 global cycles,
//                            needs 26+ ticks of sustained ≥80 % pass-rate)
//   ticks 66+      : FILL

/// Returns the distribution slot index to use for tick position `t`.
///
/// For FILL ticks the slot is zone-specific, based on how many fill_head
/// advances are expected by that tick position.  Each zone spans 8 values
/// of fill_head; with accumulators ready after ~7 ticks and an advance every
/// 4 ticks, traversing one zone takes roughly 32 FILL ticks.
#[inline]
fn phase_for_tick(t: usize) -> usize {
    match t {
        0       => 0, // IDLE
        1..=25  => 1, // PRIME
        26..=65 => 2, // FLOW
        _ => {
            let fill_tick = t - 66;
            let est_zone  = (fill_tick / 32).min(7);
            FILL_ZONE_BASE + est_zone // slots 4-11
        }
    }
}

// ─── Seed helpers ─────────────────────────────────────────────────────────────

fn append_ticks(buf: &mut Vec<u8>, n: usize, pump: u8, valve: u8, temp: u8,
                bp: u8, conc: u8, cool: u8, cmd: u8) {
    for _ in 0..n {
        buf.extend_from_slice(&[pump, valve, temp, bp, conc, cool, cmd]);
    }
}

/// Hand-crafted seeds that already encode the correct byte ranges for each
/// phase.  These bootstrap the CEM corpus so that:
///   – Early generations see PRIME / FLOW / FILL ticks immediately.
///   – The CEM has high-quality prefix sequences to extend rather than
///     having to discover them from scratch.
///   – The full-pipeline seed covers all 8 FILL zones and triggers the OOB
///     crash, giving LibAFL an immediate objective and giving the CEM a
///     crash-triggering reference sequence to learn variations of.
///
/// FILL zone timing (with the bytes below, accumulators are always ready):
///   fill_head advances every 4 FILL ticks once accumulators saturate (~7 ticks).
///   Zone boundary FILL ticks: 35, 67, 99, 131, 163, 195, 227, 259 (crash).
///   byte switch happens at exactly the tick where fill_head enters the new zone,
///   because the zone check runs on the POST-advance fill_head value.
fn bootstrap_seeds() -> Vec<BytesInput> {
    let mut seeds = Vec::new();

    // Seed 1: IDLE → PRIME only.
    let mut s1 = Vec::new();
    append_ticks(&mut s1, 1, 0, 0, 0, 0, 0, 0, 1);
    seeds.push(BytesInput::new(s1));

    // Seed 2: IDLE → PRIME → FLOW.
    let mut s2 = Vec::new();
    append_ticks(&mut s2,  1, 0,  0,  0,  0,  0,  0, 1);  // IDLE
    append_ticks(&mut s2, 24, 60, 30,  0, 60,  0,  0, 0);  // PRIME  pump+bp=120, valve=30
    seeds.push(BytesInput::new(s2));

    // Seed 3: IDLE → PRIME → FLOW → FILL zone-0 start.
    let mut s3 = Vec::new();
    append_ticks(&mut s3,  1,  0,  0,  0,  0,  0,  0, 1);
    append_ticks(&mut s3, 24, 60, 30,  0, 60,  0,  0, 0);  // PRIME
    append_ticks(&mut s3, 12, 60, 30, 60, 60, 40, 60, 0);  // FLOW   temp+cool=120, conc=40
    append_ticks(&mut s3, 35, 60, 20, 60, 50,  0,  0, 0);  // FILL zone 0: pv=80∈[60,90] temp=60∈[50,65]
    seeds.push(BytesInput::new(s3));

    // Seed 4: full pipeline – covers all 8 FILL zones and triggers the
    // fill_head≥64 OOB crash.
    //
    // Input byte roles in FILL phase:
    //   [0] pump_rate   – contributes to flow_accum (needs [40,90]) AND zone pv (pump+valve)
    //   [1] valve_pos   – contributes to zone pv only (pump+valve)
    //   [2] pipe_temp   – contributes to temp_accum (needs [50,100]) AND zone temp
    //   [3] back_press  – contributes to press_accum (needs [30,80])
    //   [4] feed_conc   – irrelevant in FILL
    //   [5] coolant     – irrelevant in FILL
    //   [6] cmd         – irrelevant in FILL (0)
    //
    // Zone byte selection (all satisfy both accumulator AND zone validity):
    //   zone 0: pv∈[60, 90]  temp∈[50,65]  → pump=60 valve=20 (pv=80) temp=60 bp=50
    //   zone 1: pv∈[80,110]  temp∈[62,77]  → pump=80 valve=20 (pv=100) temp=70 bp=50
    //   zone 2: pv∈[70,100]  temp∈[72,87]  → pump=65 valve=20 (pv=85)  temp=80 bp=50
    //   zone 3: pv∈[55, 85]  temp∈[65,80]  → pump=50 valve=20 (pv=70)  temp=72 bp=50
    //   zone 4: pv∈[95,125]  temp∈[55,70]  → pump=90 valve=20 (pv=110) temp=62 bp=50
    //   zone 5: pv∈[65, 95]  temp∈[78,93]  → pump=60 valve=20 (pv=80)  temp=85 bp=50
    //   zone 6: pv∈[85,115]  temp∈[52,67]  → pump=80 valve=20 (pv=100) temp=60 bp=50
    //   zone 7: pv∈[75,105]  temp∈[63,78]  → pump=70 valve=20 (pv=90)  temp=70 bp=50
    let mut s4 = Vec::new();
    // Pre-FILL (37 ticks).
    append_ticks(&mut s4,  1,  0,  0,  0,  0,  0,  0, 1);  // IDLE
    append_ticks(&mut s4, 24, 60, 30,  0, 60,  0,  0, 0);  // PRIME
    append_ticks(&mut s4, 12, 60, 30, 60, 60, 40, 60, 0);  // FLOW
    // FILL: zone transitions at fill-phase ticks 35, 67, 99, 131, 163, 195, 227.
    //   zone 0: FILL ticks  0-34 (35 ticks, fill_head 0→7, switch at tick 35)
    append_ticks(&mut s4, 35, 60, 20, 60, 50, 0, 0, 0);
    //   zone 1: FILL ticks 35-66 (32 ticks, fill_head 8→15, switch at tick 67)
    append_ticks(&mut s4, 32, 80, 20, 70, 50, 0, 0, 0);
    //   zone 2: FILL ticks 67-98 (32 ticks)
    append_ticks(&mut s4, 32, 65, 20, 80, 50, 0, 0, 0);
    //   zone 3: FILL ticks 99-130
    append_ticks(&mut s4, 32, 50, 20, 72, 50, 0, 0, 0);
    //   zone 4: FILL ticks 131-162
    append_ticks(&mut s4, 32, 90, 20, 62, 50, 0, 0, 0);
    //   zone 5: FILL ticks 163-194
    append_ticks(&mut s4, 32, 60, 20, 85, 50, 0, 0, 0);
    //   zone 6: FILL ticks 195-226
    append_ticks(&mut s4, 32, 80, 20, 60, 50, 0, 0, 0);
    //   zone 7: FILL ticks 227-259 → fill_head reaches 64 → abort()
    append_ticks(&mut s4, 33, 70, 20, 70, 50, 0, 0, 0);
    seeds.push(BytesInput::new(s4));

    seeds
}

// ─── CEM data structures ──────────────────────────────────────────────────────

/// One tick's bytes + the actual PLC phase that was active when they ran.
struct TickRecord {
    phase: usize,
    bytes: [u8; TICK_SIZE],
}

/// One complete episode.
struct Episode {
    ticks:  Vec<TickRecord>,
    reward: f32,
}

/// All learnable CEM state.
struct Cem {
    /// dists[phase][byte_pos][byte_val] = probability (sums to 1 per [p][pos]).
    dists: [[[f32; 256]; TICK_SIZE]; NUM_PHASES],

    /// Completed episodes accumulating toward the next update.
    buffer: Vec<Episode>,

    /// Set by the harness after each run; consumed by post_exec.
    pending: Option<Episode>,

    /// Cross-generation novelty: every fill_head value ever observed.
    seen_fill_heads: HashSet<i32>,
    best_fill_head:  i32,

    generation:     usize,
    total_episodes: usize,
}

impl Cem {
    fn new() -> Self {
        let uniform = 1.0_f32 / 256.0;
        Cem {
            dists:           [[[uniform; 256]; TICK_SIZE]; NUM_PHASES],
            buffer:          Vec::with_capacity(POPULATION_SIZE + 8),
            pending:         None,
            seen_fill_heads: HashSet::new(),
            best_fill_head:  -1,
            generation:      0,
            total_episodes:  0,
        }
    }

    /// Inverse-CDF sample: draw one byte from a normalised distribution.
    /// `r` must be in [0, 1].
    fn sample_byte(dist: &[f32; 256], r: f64) -> u8 {
        let mut cum = 0.0_f64;
        for (b, &p) in dist.iter().enumerate() {
            cum += p as f64;
            if r <= cum {
                return b as u8;
            }
        }
        255 // floating-point rounding fallback
    }

    /// Update distributions from the current elite set, then clear the buffer.
    fn update(&mut self) {
        let n = self.buffer.len();
        if n == 0 { return; }

        // Sort descending by reward.
        self.buffer.sort_unstable_by(|a, b| {
            b.reward.partial_cmp(&a.reward)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let elite_n    = ELITE_COUNT.min(n);
        let best_reward = self.buffer[0].reward;

        // Count byte-value occurrences per (phase, byte_pos) in elite ticks.
        let mut counts = [[[0u32; 256]; TICK_SIZE]; NUM_PHASES];
        let mut totals = [[0u32; TICK_SIZE]; NUM_PHASES];

        for ep in &self.buffer[..elite_n] {
            for tick in &ep.ticks {
                let ph = tick.phase;
                for pos in 0..TICK_SIZE {
                    counts[ph][pos][tick.bytes[pos] as usize] += 1;
                    totals[ph][pos] += 1;
                }
            }
        }

        // EMA blend: dist = (1-lr)*dist + lr*elite_freq, then normalise.
        for ph in 0..NUM_PHASES {
            for pos in 0..TICK_SIZE {
                let tot = totals[ph][pos];
                if tot == 0 { continue; }
                let tot_f = tot as f32;
                for b in 0..256usize {
                    let elite_freq = counts[ph][pos][b] as f32 / tot_f;
                    self.dists[ph][pos][b] =
                        (1.0 - LEARNING_RATE) * self.dists[ph][pos][b]
                        + LEARNING_RATE * elite_freq;
                    if self.dists[ph][pos][b] < MIN_PROB {
                        self.dists[ph][pos][b] = MIN_PROB;
                    }
                }
                // Renormalise.
                let sum: f32 = self.dists[ph][pos].iter().sum();
                if sum > 0.0 {
                    for b in 0..256usize {
                        self.dists[ph][pos][b] /= sum;
                    }
                }
            }
        }

        println!(
            "[CEM] gen={:>4}  episodes={:>3}  best_reward={:>8.1}  best_fill_head={}",
            self.generation, n, best_reward, self.best_fill_head
        );
        self.generation += 1;
        self.buffer.clear();
    }
}

// ─── Global CEM pointer ───────────────────────────────────────────────────────
//
// LibAFL's in-process executor is single-threaded.  Raw pointer access is safe.

static mut CEM_PTR: *mut Cem = std::ptr::null_mut();

#[inline(always)]
unsafe fn cem() -> &'static mut Cem { unsafe { &mut *CEM_PTR } }

// ─── CEM Mutator ──────────────────────────────────────────────────────────────

pub struct CemMutator;

impl Named for CemMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("CemMutator");
        &N
    }
}

/// Sample one tick (TICK_SIZE bytes) from the CEM distribution for `phase`.
fn sample_tick_with_rand<R: Rand>(rand: &mut R, cem: &Cem, phase: usize) -> [u8; TICK_SIZE] {
    let mut tb = [0u8; TICK_SIZE];
    for pos in 0..TICK_SIZE {
        let r = rand.next() as f64 / (u64::MAX as f64 + 1.0);
        tb[pos] = Cem::sample_byte(&cem.dists[phase][pos], r);
    }
    tb
}

impl<S> Mutator<BytesInput, S> for CemMutator
where
    S: HasRand,
{
    /// Either extend the current corpus entry (60 %) or generate a fresh
    /// episode from scratch (40 %).  Both modes use CEM distributions for
    /// all ticks that they sample.
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut BytesInput,
    ) -> Result<MutationResult, libafl::Error> {
        let cem  = unsafe { cem() };
        let mode = state.rand_mut().below_or_zero(10); // 0-5 = extend, 6-9 = fresh

        if mode < 6 {
            // ── Mode A: extend an existing corpus entry ────────────────────
            //
            // Keep a prefix of the current input (up to 90 % of its ticks),
            // then append freshly-sampled ticks up to MAX_TICKS.
            let existing = input.target_bytes().as_slice().to_vec();
            let total_ticks = existing.len() / TICK_SIZE;

            if total_ticks > 0 {
                let max_keep   = (total_ticks * 9 / 10).max(1);
                let keep_ticks = 1 + state.rand_mut().below_or_zero(max_keep);
                let prefix_end = keep_ticks * TICK_SIZE;
                let extend_max = MAX_TICKS.saturating_sub(keep_ticks);
                let extend_n   = state.rand_mut().below_or_zero(extend_max + 1);

                let mut bytes = existing[..prefix_end].to_vec();
                // Estimate the zone by inspecting the last byte of the prefix.
                // The last tick's pump_rate+valve_pos gives a rough pv estimate
                // but we don't know fill_head without running the PLC.
                // Use the FILL-generic dist (3) for extension – the harness
                // will re-attribute to the correct zone distribution during learning.
                for t in 0..extend_n {
                    let abs_t = keep_ticks + t;
                    let ph = phase_for_tick(abs_t); // 3 = FILL-generic for ticks 66+
                    bytes.extend_from_slice(&sample_tick_with_rand(
                        state.rand_mut(), cem, ph,
                    ));
                }

                if bytes.len() >= TICK_SIZE {
                    *input = BytesInput::new(bytes);
                    return Ok(MutationResult::Mutated);
                }
            }
            // Fall through to fresh mode if entry was too short.
        }

        // ── Mode B: fresh episode ──────────────────────────────────────────
        let range     = MAX_TICKS - MIN_TICKS + 1;
        let num_ticks = MIN_TICKS + state.rand_mut().below_or_zero(range);
        let mut bytes = Vec::with_capacity(num_ticks * TICK_SIZE);

        for t in 0..num_ticks {
            if t == 0 {
                bytes.extend_from_slice(&[0u8, 0, 0, 0, 0, 0, 1]); // cmd=1
            } else {
                let ph = phase_for_tick(t);
                bytes.extend_from_slice(&sample_tick_with_rand(
                    state.rand_mut(), cem, ph,
                ));
            }
        }

        *input = BytesInput::new(bytes);
        Ok(MutationResult::Mutated)
    }

    /// Called by LibAFL after execution.  Moves the pending episode record
    /// into the generation buffer, and triggers a distribution update when full.
    fn post_exec(
        &mut self,
        _state: &mut S,
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        let cem = unsafe { cem() };
        if let Some(ep) = cem.pending.take() {
            cem.buffer.push(ep);
            cem.total_episodes += 1;
            if cem.buffer.len() >= POPULATION_SIZE {
                cem.update();
            }
        }
        Ok(())
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn run() {
    println!("🤖 Starting CEM-RL Greybox Fuzzer");
    println!("   population={POPULATION_SIZE}  elites={ELITE_COUNT}  \
              lr={LEARNING_RATE}  min_ticks={MIN_TICKS}  max_ticks={MAX_TICKS}");
    println!("   Mutation: 60% extend-corpus  40% fresh-episode");
    println!("   Reward: phase=+15  prime/flux_score_Δ=+2  accum_Δ=+0.5  fill_head_Δ=+3  novelty=+10  tick=-0.001");

    // ── Heap-allocate CEM state ───────────────────────────────────────────────
    unsafe { CEM_PTR = Box::into_raw(Box::new(Cem::new())); }

    unsafe { harness_boot_plc(); }
    let input_size = unsafe { plc_get_input_size() };
    assert_eq!(input_size, TICK_SIZE,
        "Target tick size is {input_size} but TICK_SIZE={TICK_SIZE}");

    // ── Observers ────────────────────────────────────────────────────────────
    let edges_observer = unsafe { std_edges_map_observer("edges") };

    let metrics_slice = unsafe {
        core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(CEM_METRICS) as *mut u8,
            METRICS_SIZE,
        )
    };
    let metrics_observer = unsafe { StdMapObserver::new("cem_metrics", metrics_slice) };

    // ── Feedback ─────────────────────────────────────────────────────────────
    let mut feedback = feedback_or!(
        MaxMapFeedback::new(&edges_observer),
        MaxMapFeedback::new(&metrics_observer)
    );
    let mut objective = CrashFeedback::new();

    // ── Fuzzer state ─────────────────────────────────────────────────────────
    let mut fuzz_state = StdState::new(
        StdRand::with_seed(42),
        InMemoryCorpus::new(),
        OnDiskCorpus::new("crashes").unwrap(),
        &mut feedback,
        &mut objective,
    )
    .unwrap();

    let scheduler = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let mut stages = tuple_list!(StdMutationalStage::new(CemMutator));

    let monitor = SimpleMonitor::new(|s| println!("{s}"));
    let mut mgr  = SimpleEventManager::new(monitor);

    // ── Harness closure ───────────────────────────────────────────────────────
    //
    // Executes every tick individually so we can observe the PLC state change
    // per tick and accumulate the dense reward signal for CEM.
    let mut harness = |input: &BytesInput| {
        let buf = input.target_bytes();
        let buf = buf.as_slice();

        unsafe {
            harness_reset_plc();

            let mp = core::ptr::addr_of_mut!(CEM_METRICS);
            (*mp).fill(0);

            let aligned = buf.len() - (buf.len() % TICK_SIZE);
            if aligned == 0 {
                return libafl::executors::ExitKind::Ok;
            }

            let c = cem();

            let mut prev = PipelineState::default();
            plc_get_full_state(&mut prev as *mut _ as *mut u8, size_of::<PipelineState>());

            let mut max_fh:   i32 = 0;
            let mut ep_reward: f32 = 0.0;
            let mut ep_ticks: Vec<TickRecord> = Vec::with_capacity(aligned / TICK_SIZE);

            for chunk in buf[..aligned].chunks_exact(TICK_SIZE) {
                plc_step(chunk.as_ptr(), TICK_SIZE);

                let mut curr = PipelineState::default();
                plc_get_full_state(&mut curr as *mut _ as *mut u8, size_of::<PipelineState>());

                // ── Per-tick reward ───────────────────────────────────────────

                let mut r = -0.001_f32;

                // Phase transition.
                if curr.phase > prev.phase {
                    r += 15.0 * (curr.phase - prev.phase) as f32;
                }

                // PRIME phase progress: reward prime_score increases so the CEM
                // can distinguish good PRIME bytes from bad ones before the
                // PRIME→FLOW transition ever fires.
                let ps = curr.prime_score - prev.prime_score;
                if ps > 0 { r += 2.0 * ps as f32; }

                // FLOW phase progress: same idea for flux_score.
                let fs = curr.flux_score - prev.flux_score;
                if fs > 0 { r += 2.0 * fs as f32; }

                // FILL accumulator readiness – tiny per-tick signal so the
                // CEM learns which channels build accumulators, but capped to
                // prevent this from dominating fill_head progress rewards.
                let fa = (curr.flow_accum  - prev.flow_accum).clamp(0, 4);
                let pa = (curr.press_accum - prev.press_accum).clamp(0, 4);
                let ta = (curr.temp_accum  - prev.temp_accum).clamp(0, 4);
                r += 0.05 * (fa + pa + ta) as f32;

                // fill_head advancement.
                let fh_d = curr.fill_head - prev.fill_head;
                if fh_d > 0 { r += 3.0 * fh_d as f32; }

                // Novelty: reward any fill_head value never seen before.
                let fh = curr.fill_head;
                if fh > 0 && !c.seen_fill_heads.contains(&fh) {
                    c.seen_fill_heads.insert(fh);
                    r += 10.0;
                    if fh > c.best_fill_head {
                        c.best_fill_head = fh;
                        println!(
                            "[CEM] ★ new best fill_head={fh:<3}  \
                             gen={:<4}  total_ep={}",
                            c.generation, c.total_episodes
                        );
                    }
                }

                max_fh = max_fh.max(curr.fill_head.max(0));
                ep_reward += r;

                // Attribution: actual PLC phase for IDLE/PRIME/FLOW.
                // For FILL, use the zone-specific distribution (fill_head / 8)
                // so the CEM learns separate byte values per zone.
                let ph = if curr.phase == 3 {
                    FILL_ZONE_BASE + (curr.fill_head.max(0) as usize / 8).min(7)
                } else {
                    curr.phase.min(3) as usize
                };
                let mut tb = [0u8; TICK_SIZE];
                tb.copy_from_slice(chunk);
                ep_ticks.push(TickRecord { phase: ph, bytes: tb });

                prev = curr;
            }

            // ── Update LibAFL metrics ─────────────────────────────────────────
            (*mp)[0] = prev.phase.min(3) as u8;
            (*mp)[1] = prev.prime_score.clamp(0, 255) as u8;
            (*mp)[2] = prev.flux_score.clamp(0, 255) as u8;
            (*mp)[3] = max_fh.min(255) as u8;

            // ── Store episode for post_exec ───────────────────────────────────
            c.pending = Some(Episode { ticks: ep_ticks, reward: ep_reward });
        }

        libafl::executors::ExitKind::Ok
    };

    // ── Executor ─────────────────────────────────────────────────────────────
    let mut executor = InProcessExecutor::new(
        &mut harness,
        tuple_list!(edges_observer, metrics_observer),
        &mut fuzzer,
        &mut fuzz_state,
        &mut mgr,
    )
    .unwrap();

    // ── Seed corpus ───────────────────────────────────────────────────────────
    // Bootstrap with hand-crafted sequences that already encode correct byte
    // ranges for each phase.  The CEM then extends these sequences rather than
    // rediscovering PRIME / FLOW from scratch.
    for seed in bootstrap_seeds() {
        fuzzer.add_input(&mut fuzz_state, &mut executor, &mut mgr, seed).unwrap();
    }

    println!("[CEM] Fuzz loop started.  Each '★' line is a new fill_head record.");
    println!("[CEM] Crashes saved to ./crashes/");
    println!();

    fuzzer
        .fuzz_loop(&mut stages, &mut executor, &mut fuzz_state, &mut mgr)
        .unwrap();
}
