//! Manual release-mode microbenchmarks for complete v1 signature envelopes.
//!
//! Run explicitly with `cargo test --release --locked --test providers --
//! --ignored --nocapture`. Key generation and warmup are excluded from samples.
//! This measures public StateChronicle provider/context/codec paths, not isolated
//! hardware instructions. See the conformance README for interpretation.

use std::{hint::black_box, time::Instant};

use super::{
    AwsLcMlDsa65Signer, AwsLcMlDsa65Verifier, MlDsa65Signer, MlDsa65Verifier,
    SlhDsaSha2_128sSigner, SlhDsaSha2_128sVerifier,
};
use statechronicle_core::{
    signature_hybrid::{
        HYBRID_SIGNATURE_BYTES, HybridSigner, HybridSignerMember, HybridVerifier,
        HybridVerifierMember,
    },
    signature_operation::SignatureContext,
    signature_provider::{
        Ed25519Signer, Ed25519Verifier, KeySetId, SignaturePurpose, Signer, Verifier,
    },
    signature_wire::decode_v1_envelope,
};

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn percentile(sorted_ns: &[u128], percentile: usize) -> u128 {
    let rank = sorted_ns.len().saturating_mul(percentile).div_ceil(100);
    sorted_ns[rank.saturating_sub(1)]
}

fn report(
    suite: &str,
    operation: &str,
    samples_ns: &mut [u128],
    body_bytes: usize,
    signature_bytes: usize,
    envelope_bytes: usize,
) {
    samples_ns.sort_unstable();
    let count = samples_ns.len();
    println!(
        "{suite},{operation},count={count},body_bytes={body_bytes},signature_bytes={signature_bytes},envelope_bytes={envelope_bytes},p50_ns={},p95_ns={},p99_ns={}",
        percentile(samples_ns, 50),
        percentile(samples_ns, 95),
        percentile(samples_ns, 99),
    );
}

#[derive(Clone, Copy)]
struct Workload<'input> {
    body: &'input [u8],
    warmup: usize,
    count: usize,
}

fn benchmark_suite(
    suite_name: &str,
    context: &SignatureContext<'_>,
    signer: &dyn Signer,
    verifier: &dyn Verifier,
    workload: Workload<'_>,
) {
    let Workload {
        body,
        warmup,
        count,
    } = workload;
    let wire_record = context
        .sign_envelope(body, signer)
        .expect("fixture signature");
    let envelope = decode_v1_envelope(&wire_record, context.purpose).expect("fixture envelope");
    let signature_bytes = envelope.signature.len();
    let envelope_bytes = wire_record.len();

    let mut sign_samples = Vec::with_capacity(count);
    for index in 0..warmup + count {
        let start = Instant::now();
        let signed = context
            .sign_envelope(black_box(body), signer)
            .expect("envelope signing");
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(signed.len(), envelope_bytes, "wire size changed");
        black_box(signed);
        if index >= warmup {
            sign_samples.push(elapsed);
        }
    }
    report(
        suite_name,
        "sign_envelope",
        &mut sign_samples,
        body.len(),
        signature_bytes,
        envelope_bytes,
    );

    let mut verify_samples = Vec::with_capacity(count);
    for index in 0..warmup + count {
        let start = Instant::now();
        let parsed =
            decode_v1_envelope(black_box(&wire_record), context.purpose).expect("decode envelope");
        context
            .verify_envelope(parsed, verifier)
            .expect("envelope verification");
        let elapsed = start.elapsed().as_nanos();
        if index >= warmup {
            verify_samples.push(elapsed);
        }
    }
    report(
        suite_name,
        "decode_verify_envelope",
        &mut verify_samples,
        body.len(),
        signature_bytes,
        envelope_bytes,
    );
}

#[test]
#[ignore = "manual release-mode benchmark; run using the README command"]
fn release_v1_signature_microbenchmarks() {
    let count = env_count("SC_BENCH_SAMPLES", 100);
    let warmup = env_count("SC_BENCH_WARMUP", 10);
    let body = b"StateChronicle v1 release microbenchmark body";
    let workload = Workload {
        body,
        warmup,
        count,
    };
    let purpose = SignaturePurpose::TenantCommit;

    println!(
        "environment,arch={},os={},samples={count},warmup={warmup},timer=std::time::Instant",
        std::env::consts::ARCH,
        std::env::consts::OS,
    );
    println!("suite,operation,details");

    let ed_key_id = KeySetId([2; 32]);
    let ed_private = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let ed_signer = Ed25519Signer::new(&ed_private, ed_key_id, purpose);
    let ed_verifier = Ed25519Verifier::new(&ed_private.verifying_key(), ed_key_id, purpose);
    let ed_context = SignatureContext {
        suite: ed_signer.suite(),
        key_set_id: &ed_key_id,
        purpose,
        body_schema: "statechronicle.commit.v0",
        signature_len: 64,
    };
    benchmark_suite("ed25519", &ed_context, &ed_signer, &ed_verifier, workload);

    let rust_key_id = KeySetId([3; 32]);
    use ml_dsa::Keypair as _;
    let seed: ml_dsa::Seed = [0x42; 32].into();
    let rust_private = ml_dsa::SigningKey::<ml_dsa::MlDsa65>::from_seed(&seed);
    let rust_public = rust_private.verifying_key();
    let rust_signer = MlDsa65Signer::new(rust_private, rust_key_id, purpose);
    let rust_verifier = MlDsa65Verifier::new(rust_public, rust_key_id, purpose);
    let rust_context = SignatureContext {
        suite: rust_signer.suite(),
        key_set_id: &rust_key_id,
        purpose,
        body_schema: "statechronicle.commit.v0",
        signature_len: 3309,
    };
    benchmark_suite(
        "ml-dsa-65-rustcrypto",
        &rust_context,
        &rust_signer,
        &rust_verifier,
        workload,
    );

    use aws_lc_rs::signature::{KeyPair, ML_DSA_65_SIGNING, PqdsaKeyPair};
    let aws_key_id = KeySetId([4; 32]);
    let aws_private = PqdsaKeyPair::generate(&ML_DSA_65_SIGNING).expect("AWS-LC keygen");
    let aws_public = aws_private.public_key().as_ref().to_vec();
    let aws_signer = AwsLcMlDsa65Signer::new(aws_private, aws_key_id, purpose);
    let aws_verifier = AwsLcMlDsa65Verifier::new(aws_public, aws_key_id, purpose);
    let aws_context = SignatureContext {
        suite: aws_signer.suite(),
        key_set_id: &aws_key_id,
        purpose,
        body_schema: "statechronicle.commit.v0",
        signature_len: 3309,
    };
    benchmark_suite(
        "ml-dsa-65-aws-lc",
        &aws_context,
        &aws_signer,
        &aws_verifier,
        workload,
    );

    let slh_key_id = KeySetId([5; 32]);
    let mut rng = rand::rng();
    let slh_private = slh_dsa::SigningKey::<slh_dsa::Sha2_128s>::new(&mut rng);
    let slh_public = slh_private.verifying_key();
    let slh_signer = SlhDsaSha2_128sSigner::new(slh_private, slh_key_id, purpose);
    let slh_verifier = SlhDsaSha2_128sVerifier::new(slh_public, slh_key_id, purpose);
    let slh_context = SignatureContext {
        suite: slh_signer.suite(),
        key_set_id: &slh_key_id,
        purpose,
        body_schema: "statechronicle.commit.v0",
        signature_len: 7856,
    };
    benchmark_suite(
        "slh-dsa-sha2-128s-rustcrypto",
        &slh_context,
        &slh_signer,
        &slh_verifier,
        workload,
    );

    let hybrid_key_id = KeySetId([1; 32]);
    let hybrid_ed_key_id = KeySetId([6; 32]);
    let hybrid_pq_key_id = KeySetId([7; 32]);
    let hybrid_ed_private = ed25519_dalek::SigningKey::from_bytes(&[8; 32]);
    let hybrid_ed_signer = Ed25519Signer::new(&hybrid_ed_private, hybrid_ed_key_id, purpose);
    let hybrid_ed_verifier = Ed25519Verifier::new(
        &hybrid_ed_private.verifying_key(),
        hybrid_ed_key_id,
        purpose,
    );
    let hybrid_pq_private = PqdsaKeyPair::generate(&ML_DSA_65_SIGNING).expect("hybrid keygen");
    let hybrid_pq_public = hybrid_pq_private.public_key().as_ref().to_vec();
    let hybrid_pq_signer = AwsLcMlDsa65Signer::new(hybrid_pq_private, hybrid_pq_key_id, purpose);
    let hybrid_pq_verifier = AwsLcMlDsa65Verifier::new(hybrid_pq_public, hybrid_pq_key_id, purpose);
    let hybrid_signer = HybridSigner::new(
        hybrid_key_id,
        purpose,
        HybridSignerMember {
            signer: &hybrid_ed_signer,
            key_set_id: hybrid_ed_key_id,
        },
        HybridSignerMember {
            signer: &hybrid_pq_signer,
            key_set_id: hybrid_pq_key_id,
        },
    )
    .expect("hybrid signer");
    let hybrid_verifier = HybridVerifier::new(
        hybrid_key_id,
        purpose,
        HybridVerifierMember {
            verifier: std::sync::Arc::new(hybrid_ed_verifier),
            key_set_id: hybrid_ed_key_id,
        },
        HybridVerifierMember {
            verifier: std::sync::Arc::new(hybrid_pq_verifier),
            key_set_id: hybrid_pq_key_id,
        },
    )
    .expect("hybrid verifier");
    let hybrid_context = SignatureContext {
        suite: hybrid_signer.suite(),
        key_set_id: &hybrid_key_id,
        purpose,
        body_schema: "statechronicle.commit.v0",
        signature_len: HYBRID_SIGNATURE_BYTES,
    };
    benchmark_suite(
        "hybrid-ed25519-ml-dsa-65-aws-lc",
        &hybrid_context,
        &hybrid_signer,
        &hybrid_verifier,
        workload,
    );
}
