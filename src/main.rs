use core::num::NonZeroUsize;

use libafl::{
    corpus::{InMemoryCorpus, OnDiskCorpus},
    events::SimpleEventManager,
    executors::inprocess::InProcessExecutor,
    feedbacks::{MaxMapFeedback, CrashFeedback},
    fuzzer::{Fuzzer, StdFuzzer},
    generators::RandBytesGenerator,
    inputs::{BytesInput, HasTargetBytes},
    monitors::SimpleMonitor,
    mutators::{havoc_mutations::havoc_mutations, scheduled::HavocScheduledMutator},
    schedulers::QueueScheduler,
    // FIX 1: Import the Mutational Stage
    stages::StdMutationalStage,
    state::StdState,
};
use libafl_bolts::{AsSlice, rands::StdRand, tuples::tuple_list};
use libafl_targets::std_edges_map_observer;

// Define the C function interface directly to bypass the wrappers
unsafe extern "C" {
    unsafe fn LLVMFuzzerTestOneInput(data: *const u8, size: usize) -> i32;
}


fn main() {
    // 1. Observer
    let edges_observer = unsafe { std_edges_map_observer("edges") };

    // 2. Feedback
    let mut feedback = MaxMapFeedback::new(&edges_observer);
    let mut objective = CrashFeedback::new();

    // 3. State
    let mut state = StdState::new(
        StdRand::with_seed(0),
        InMemoryCorpus::new(),
        OnDiskCorpus::new("crashes").unwrap(),
        &mut feedback,
        &mut objective,
    )
    .unwrap();

    // 4. Scheduler & Mutator
    let scheduler = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let mutator = HavocScheduledMutator::new(havoc_mutations());

    // FIX 2: Wrap the mutator in a Stage, and put the Stage in a tuple list
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    // 5. Monitor & Event Manager
    let monitor = SimpleMonitor::new(|s| println!("{}", s));
    let mut mgr = SimpleEventManager::new(monitor);

    // 6. Harness
    let mut harness = |input: &BytesInput| {
        let target = input.target_bytes();
        let buf = target.as_slice();
        unsafe {
            LLVMFuzzerTestOneInput(buf.as_ptr(), buf.len());
        }
        libafl::executors::ExitKind::Ok
    };

    // 7. Executor
    let mut executor = InProcessExecutor::new(
        &mut harness,
        tuple_list!(edges_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
    )
    .unwrap();

    // 8. Initial Seed
    let mut generator = RandBytesGenerator::new(NonZeroUsize::new(64).unwrap());
    state
        .generate_initial_inputs(&mut fuzzer, &mut executor, &mut generator, &mut mgr, 1)
        .unwrap();

    // 9. Fuzz Loop
    // FIX 3: Pass `&mut stages` instead of the bare mutator
    fuzzer
        .fuzz_loop(&mut stages, &mut executor, &mut state, &mut mgr)
        .unwrap();
}
