use statechronicle::{Event, EventId, IntentId, Operation, ResourceId, SubjectId, TenantId};
use statechronicle::core::canonicalize::canonicalize_and_digest;
use statechronicle::domain::{event::StateCommitment, resource_state::{ResourceState, UniqueAssetState}, status::Status};
use statechronicle::commit::roots::event_root;
use std::{hint::black_box, time::Instant};
fn main() {
    let state = ResourceState::UniqueAsset(UniqueAssetState { owner: SubjectId("account:alice".into()), status: Status::from_static("active"), trade_id: None });
    let digest = canonicalize_and_digest(&state).unwrap();
    let events: Vec<_> = (0..256).map(|n| Event::new(
        TenantId("tenant:benchmark".into()), EventId(format!("evt_{n:04}")), IntentId(format!("int_{n:04}")),
        Operation::from_static("asset.transfer"), ResourceId(format!("asset:{n:04}")), SubjectId("account:alice".into()),
        StateCommitment { version: 1, state_hash: digest.clone(), state: state.clone() },
        StateCommitment { version: 2, state_hash: digest.clone(), state: state.clone() },
        None, SubjectId("service:benchmark".into()), "2026-11-16T00:00:00Z".parse().unwrap(),
    )).collect();
    if let Some(argument) = std::env::args().nth(1) {
        let count: usize = argument.parse().unwrap();
        if count != 0 { for _ in 0..1000 { black_box(event_root(black_box(&events[..count])).unwrap()); } }
        return;
    }
    let mut output = Vec::new();
    for (count, iterations) in [(1, 2000), (3, 1000), (256, 20)] {
        for _ in 0..100 { black_box(event_root(black_box(&events[..count])).unwrap()); }
        let root = event_root(&events[..count]).unwrap();
        let mut samples = Vec::new();
        for _ in 0..31 {
            let start = Instant::now();
            for _ in 0..iterations { black_box(event_root(black_box(&events[..count])).unwrap()); }
            samples.push(start.elapsed().as_nanos() as f64 / f64::from(iterations));
        }
        samples.sort_by(f64::total_cmp);
        output.push(format!(r#"{{"events":{count},"root":"{}","median_ns":{},"p95_ns":{},"samples_ns":{:?}}}"#, root.as_str(), samples[15], samples[29], samples));
    }
    println!("[{}]", output.join(","));
}
