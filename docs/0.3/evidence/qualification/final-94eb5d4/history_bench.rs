#[path = "common/mod.rs"]
#[allow(dead_code)]
mod common;
use async_trait::async_trait;
use statechronicle::accumulator::sparse_merkle::StateUpdate;
use statechronicle::commit::{batch::CommitBatch,builder::CommitBuilder,roots::{compute_state_root,state_root_updates},sign::{Ed25519CommitVerifier,sign_commit}};
use statechronicle::core::canonicalize::canonicalize_and_digest;
use statechronicle::domain::{commit::CommitScope,event::StateCommitment,resource_state::{ResourceState,UniqueAssetState},status::Status};
use statechronicle::index::rebuild::{CheckpointStore,ProjectionSink,verified::{HistoryPin,HistoryWitness,VerifiedHistory,verify_history}};
use statechronicle::{Commit,CommitId,ContentDigest,Event,EventId,IntentId,Operation,ProfileId,ResourceId,Signed,StateKey,StateProjection,SubjectId};
use std::{collections::BTreeMap,future::Future,sync::{Arc,Mutex},task::{Context,Poll,Waker},time::Instant};
fn ready<T>(future:impl Future<Output=T>)->T {
    let mut future=std::pin::pin!(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {Poll::Ready(v)=>v,Poll::Pending=>panic!("non-yielding memory fixture unexpectedly pending")}
}
struct Fixture {commits:Vec<Signed<Commit>>,events:Vec<Vec<Event>>,pin:HistoryPin}
fn fixture(n:usize)->Fixture {
    let mut commits:Vec<Signed<Commit>>=Vec::new();let mut events=Vec::new();let mut updates=Vec::new();
    let mut previous=ContentDigest::new(*compute_state_root(&[]).unwrap().as_bytes());
    for index in 0..n {
        let commitment=|owner:&str,status:&'static str,version| {
            let state=ResourceState::UniqueAsset(UniqueAssetState{owner:SubjectId(owner.into()),status:Status::from_static(status),trade_id:None});
            StateCommitment{version,state_hash:canonicalize_and_digest(&state).unwrap(),state}
        };
        let event=Event::new(common::tenant(),EventId::new(format!("evt_history_{index:04}")).unwrap(),IntentId::new(format!("int_history_{index:04}")).unwrap(),Operation::from_static("asset.mint"),ResourceId(format!("asset:history:{index:04}")),SubjectId("account:alice".into()),commitment("","unborn",0),commitment("account:alice","active",1),None,common::executor_subject(),common::fixed_now());
        let mut batch=CommitBatch::new(CommitScope::tenant(common::tenant()));batch.add_event(event.clone()).unwrap();
        let body=CommitBuilder::builder().scope(batch.scope().clone()).sequence((index+1)as u64).parent(commits.last().map(|v|v.body.commit_id.clone())).executor(common::executor_subject()).profile(ProfileId::new("statechronicle.profile.resource.v0".into()).unwrap()).created_at(common::fixed_now()).build(&batch,previous,&updates,||Ok(CommitId::new(format!("cmt_history_{index:04}")).unwrap())).unwrap();
        previous=body.next_state_root.clone();updates.extend(state_root_updates(batch.events()).unwrap());
        commits.push(sign_commit(&body,&common::fixed_key(),common::key_id()).unwrap());events.push(vec![event]);
    }
    let pin=HistoryPin{tenant:common::tenant(),tip_digest:canonicalize_and_digest(&commits.last().unwrap().body).unwrap()};
    Fixture{commits,events,pin}
}
fn verify(f:&Fixture)->VerifiedHistory {
    let verifier=Ed25519CommitVerifier::new(|commit:&Signed<Commit>| {
        if commit.body.scope.tenant_id!=Some(common::tenant())||commit.signature.key_id!=common::key_id(){return Err("untrusted historical signer".into());}
        Ok(common::fixed_key().verifying_key())
    });
    let witnesses:Vec<_>=f.commits.iter().zip(&f.events).map(|(commit,events)|HistoryWitness{commit,events}).collect();
    verify_history(&witnesses,&f.pin,&verifier).unwrap()
}
#[derive(Default)]struct Database {rows:BTreeMap<(String,String),StateProjection>,offsets:BTreeMap<String,usize>,fail_ack:bool}
#[derive(Default,Clone)]struct Storage(Arc<Mutex<Database>>);
#[async_trait]impl ProjectionSink for Storage {
    async fn upsert_projection(&self,p:&StateProjection)->Result<(),String>{
        let mut db=self.0.lock().unwrap();let key=(p.tenant_id.0.clone(),p.resource_id.0.clone());
        if let Some(old)=db.rows.get(&key){assert!(old.version<=p.version);if old.version==p.version{assert_eq!(old,p);}}
        db.rows.insert(key,p.clone());Ok(())
    }
}
#[async_trait]impl CheckpointStore for Storage {
    async fn load(&self,key:&str)->Result<Option<usize>,String>{Ok(self.0.lock().unwrap().offsets.get(key).copied())}
    async fn save(&self,key:&str,next:usize)->Result<(),String>{let mut db=self.0.lock().unwrap();db.offsets.insert(key.into(),next);if std::mem::take(&mut db.fail_ack){return Err("injected checkpoint ACK loss".into());}Ok(())}
}
fn rebuild(source:&VerifiedHistory,f:&Fixture){
    let first=Storage::default();first.0.lock().unwrap().fail_ack=true;
    assert!(ready(source.rebuild_resumable("bench-history",32,&first,&first)).is_err());
    let reopened=first.clone();let remaining=ready(source.rebuild_resumable("bench-history",32,&reopened,&reopened)).unwrap();
    let n=f.commits.len();assert_eq!(remaining,n.saturating_sub(32)as u64);
    let db=reopened.0.lock().unwrap();assert_eq!(db.rows.len(),n);assert_eq!(db.offsets.get(&source.checkpoint_key("bench-history").unwrap()),Some(&n));
    let updates:Vec<_>=db.rows.values().map(|p|StateUpdate::new(StateKey::for_resource(&p.tenant_id.0,&p.resource_id.0),*p.state_hash.as_bytes())).collect();
    assert_eq!(compute_state_root(&updates).unwrap().as_bytes(),f.commits.last().unwrap().body.next_state_root.as_bytes());
}
fn main(){
    let fixtures:Vec<_>=[1,32,256].into_iter().map(fixture).collect();let sources:Vec<_>=fixtures.iter().map(verify).collect();
    let fixture_digests:Vec<_>=fixtures.iter().map(|f|canonicalize_and_digest(&(&f.commits,&f.events)).unwrap().to_string()).collect();
    let only=std::env::args().nth(1).map(|mut s|{let n=s.trim_end_matches('_').len();s.truncate(n);s});
    let allocation=std::env::var("STATECHRONICLE_ALLOCATION_PROBE").ok().as_deref()==Some("1");
    let allocation_actions:usize=std::env::var("STATECHRONICLE_ALLOCATION_ACTIONS").unwrap_or_else(|_|"1000".into()).parse().unwrap();assert!((1..=1000).contains(&allocation_actions));
    let mut records=Vec::new();
    let mut run=|name:&str,count:usize,action:&mut dyn FnMut()|{
        if only.as_deref().is_some_and(|v|v!=name){return;}
        if allocation {for _ in 0..allocation_actions{action();}std::process::exit(0);}
        for _ in 0..3{action();}let mut values=Vec::with_capacity(count);
        for _ in 0..count{let start=Instant::now();action();values.push(start.elapsed().as_nanos()as u64);}
        values.sort_unstable();let n=values.len();records.push(serde_json::json!({"workload":name,"sample_count":n,"median_ns":values[n/2],"p95_ns":values[(n*95).div_ceil(100)-1],"p99_ns":values[(n*99).div_ceil(100)-1],"samples_ns":values}));
    };
    run("control",1000,&mut||{std::hint::black_box(1u8);});
    for (f,source)in fixtures.iter().zip(&sources){
        let n=f.commits.len();let count=if n==1{100}else{31};
        let names=match n{1=>("verify_signed_history_1","checkpoint_ack_loss_resume_and_root_1"),32=>("verify_signed_history_32","checkpoint_ack_loss_resume_and_root_32"),256=>("verify_signed_history_256","checkpoint_ack_loss_resume_and_root_256"),_=>unreachable!()};
        run(names.0,count,&mut||{let checked=verify(f);std::hint::black_box(checked);});
        run(names.1,count,&mut||rebuild(source,f));
    }
    println!("{}",serde_json::json!({"workloads":records,"fixture_digests":fixture_digests}));
}
