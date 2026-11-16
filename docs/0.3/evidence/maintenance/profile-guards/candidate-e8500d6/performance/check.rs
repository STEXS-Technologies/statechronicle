mod fixture;
use std::hint::black_box;
use std::time::Instant;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
struct Count;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed); BYTES.fetch_add(l.size() as u64, Relaxed);
        unsafe {System.alloc(l)}
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe {System.dealloc(p,l)} }
    unsafe fn realloc(&self, p:*mut u8, l:Layout, n:usize)->*mut u8 {
        ALLOCS.fetch_add(1,Relaxed); BYTES.fetch_add(n as u64,Relaxed);
        unsafe {System.realloc(p,l,n)}
    }
}
#[global_allocator] static A: Count = Count;
fn main() {
    let registry = statechronicle::ProfileRegistry::baseline();
    let all = fixture::cases();
    let labels = [
        "FungibleBalance/balance.debit/2/3/0",
        "FungibleBalance/balance.debit/2/2/0",
        "FungibleBalance/balance.debit/2/5/0",
        "ConsumableStack/stack.consume/2/3/0",
        "ConsumableStack/stack.reserve/2/5/0",
        "UniqueAsset/asset.lock/1/0",
        "UniqueAsset/asset.lock/0/0",
        "Entitlement/entitlement.activate/12/0",
        "Listing/listing.cancel/3/0",
        "Escrow/escrow.release/2/0",
    ];
    let mut rows=Vec::new();
    for label in labels {
        let c=all.iter().find(|c|c.label==label).expect("frozen case exists");
        let rules=registry.get(c.kind).unwrap();
        let check=||rules.check(black_box(&c.operation),black_box(c.current.as_ref()),black_box(&c.inputs));
        let result=format!("{:?}",check());
        for _ in 0..10000 {drop(black_box(check()));}
        let mut ns=Vec::new();
        for _ in 0..31 {
            let start=Instant::now();
            for _ in 0..50000 {drop(black_box(check()));}
            ns.push(start.elapsed().as_nanos() as f64/50000.0);
        }
        let a=ALLOCS.load(Relaxed); let b=BYTES.load(Relaxed);
        for _ in 0..1000 {drop(black_box(check()));}
        let allocations=ALLOCS.load(Relaxed)-a; let bytes=BYTES.load(Relaxed)-b;
        rows.push(serde_json::json!({"case":label,"result":result,"sample_mean_ns":ns,"allocation_calls_per_1000":allocations,"allocation_bytes_per_1000":bytes}));
    }
    println!("{}",serde_json::json!(rows));
}
