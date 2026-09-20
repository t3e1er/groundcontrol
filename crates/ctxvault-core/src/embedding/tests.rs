use super::*;

#[test]
fn test_model_name_parsing() {
    assert_eq!(
        ModelName::from_str_name("jina-embeddings-v2-base-code"),
        Some(ModelName::JinaEmbeddingsV2BaseCode)
    );
    assert_eq!(
        ModelName::from_str_name("jina-embeddings-v2-base-code-int8"),
        Some(ModelName::JinaEmbeddingsV2BaseCode)
    );
    assert_eq!(
        ModelName::from_str_name("jina-code-int8"),
        Some(ModelName::JinaEmbeddingsV2BaseCode)
    );
    assert_eq!(
        ModelName::from_str_name("jinaai/jina-embeddings-v2-base-code"),
        Some(ModelName::JinaEmbeddingsV2BaseCode)
    );
    assert_eq!(ModelName::from_str_name("jina-code"), Some(ModelName::JinaEmbeddingsV2BaseCode));
    assert_eq!(ModelName::from_str_name("jina"), Some(ModelName::JinaEmbeddingsV2BaseCode));
    assert_eq!(ModelName::from_str_name("unknown-model"), None);
}

#[test]
fn test_default_model_is_jina() {
    assert_eq!(ModelName::default(), ModelName::JinaEmbeddingsV2BaseCode);
}

#[test]
fn test_dimensions() {
    assert_eq!(ModelName::JinaEmbeddingsV2BaseCode.dimensions(), 768);
}

#[test]
fn test_average_embeddings_empty() {
    assert_eq!(average_embeddings(&[]), None);
}

#[test]
fn test_average_embeddings_single() {
    let emb = vec![1.0, 0.0, 0.0];
    let result = average_embeddings(&[emb]).unwrap();
    assert!((result[0] - 1.0).abs() < 1e-5);
    assert!((result[1] - 0.0).abs() < 1e-5);
    assert!((result[2] - 0.0).abs() < 1e-5);
}

#[test]
fn test_average_embeddings_multiple() {
    let embs = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
    let result = average_embeddings(&embs).unwrap();
    let expected_val = 1.0 / 2.0_f32.sqrt();
    assert!((result[0] - expected_val).abs() < 1e-4);
    assert!((result[1] - expected_val).abs() < 1e-4);
    assert!((result[2] - 0.0).abs() < 1e-5);
}

#[test]
fn test_aimd_controller_additive_increase() {
    let mut aimd = AimdController::new(100);
    assert!((aimd.scale - 1.0).abs() < 1e-5);

    // Fast dispatch (< 50ms): additive increase +0.10
    aimd.record_dispatch(40);
    assert!((aimd.scale - 1.10).abs() < 1e-5);
    assert!((aimd.ema_latency_ms - 40.0).abs() < 1e-5);

    aimd.record_dispatch(30);
    assert!((aimd.scale - 1.20).abs() < 1e-5);
}

#[test]
fn test_aimd_controller_multiplicative_decrease() {
    let mut aimd = AimdController::new(100);
    // Slow dispatch (> 150ms): multiplicative decrease * 0.80
    aimd.record_dispatch(200);
    assert!((aimd.scale - 0.80).abs() < 1e-5);

    aimd.record_dispatch(180);
    assert!((aimd.scale - 0.64).abs() < 1e-5);
}

#[test]
fn test_aimd_controller_sweet_spot() {
    let mut aimd = AimdController::new(100);
    // Sweet spot dispatch (between 50ms and 150ms): maintains current scale
    aimd.record_dispatch(80);
    assert!((aimd.scale - 1.0).abs() < 1e-5);

    aimd.record_dispatch(120);
    assert!((aimd.scale - 1.0).abs() < 1e-5);
}

#[test]
fn test_aimd_controller_clamping() {
    let mut aimd = AimdController::new(100);
    for _ in 0..50 {
        aimd.record_dispatch(20);
    }
    assert!(aimd.scale <= 4.0);

    for _ in 0..50 {
        aimd.record_dispatch(300);
    }
    assert!(aimd.scale >= 0.2);
}

#[test]
fn test_cpu_governor_defaults_and_l3_capping() {
    let gov = CpuGovernor::new();
    assert_eq!(gov.provider_name(), "CPU");
    assert!(gov.total_memory_bytes() > 0);
    assert!(gov.available_memory_bytes() > 0);

    let short_batch = gov.compute_adaptive_batch(128, 0);
    assert!(short_batch <= 16);
    assert!(short_batch >= 1);

    let long_batch = gov.compute_adaptive_batch(1024, 0);
    assert!(long_batch <= short_batch);
    assert!(long_batch >= 1);
}

#[test]
#[cfg(target_os = "windows")]
fn test_directml_governor_adaptive_scaling() {
    let gov = DirectMlGovernor::with_vram_bytes(8 * 1024 * 1024 * 1024);
    assert_eq!(gov.provider_name(), "DirectML");
    assert_eq!(gov.total_memory_bytes(), 8 * 1024 * 1024 * 1024);

    let base_batch = gov.compute_adaptive_batch(128, 0);
    assert!(base_batch >= 16);

    // Record fast dispatches -> batch size increases
    let boosted_batch = gov.compute_adaptive_batch(128, 30);
    assert!(boosted_batch >= base_batch);

    // Record slow dispatches -> batch size decreases
    for _ in 0..5 {
        let _ = gov.compute_adaptive_batch(128, 250);
    }
    let throttled_batch = gov.compute_adaptive_batch(128, 250);
    assert!(throttled_batch < boosted_batch);
}

#[test]
#[cfg(target_os = "windows")]
fn test_auto_select_directml_gpu() {
    let device_id = select_directml_device_id();
    println!(">>> Auto-selected DirectML Device ID: {device_id}");
    assert!(device_id >= 0);
}

#[test]
#[cfg(target_os = "windows")]
fn test_directml_device_candidates() {
    let candidates = directml_device_candidates();
    println!(">>> DirectML Candidates: {candidates:?}");
    assert!(!candidates.is_empty());
    assert!(candidates.contains(&0));
}

#[test]
#[cfg(target_os = "windows")]
fn test_detect_gpu_vram_mb() {
    let vram = detect_gpu_vram_mb();
    println!(">>> Detected GPU VRAM: {vram} MB");
    assert!(vram > 0);
}

#[test]
#[cfg(target_os = "windows")]
fn test_is_discrete_gpu_name() {
    assert!(governor::is_discrete_gpu_name("NVIDIA GeForce GTX 1070"));
    assert!(governor::is_discrete_gpu_name("NVIDIA RTX 4090"));
    assert!(governor::is_discrete_gpu_name("AMD Radeon RX 7900 XTX"));
    assert!(governor::is_discrete_gpu_name("Intel(R) Arc(TM) A770 Graphics"));
    assert!(!governor::is_discrete_gpu_name("Intel(R) HD Graphics 530"));
    assert!(!governor::is_discrete_gpu_name("Microsoft Basic Display Adapter"));
    assert!(!governor::is_discrete_gpu_name("Remote Desktop Display Adapter"));
}
