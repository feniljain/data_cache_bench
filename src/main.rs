use std::time::{Duration, Instant};

use foyer::{
    BlockEngineConfig, DeviceBuilder, FsDeviceBuilder, HybridCache, HybridCacheBuilder,
    HybridCachePolicy, HybridCacheProperties, LfuConfig, Location, RecoverMode,
};
use mixtrics::registry::prometheus_0_14::PrometheusMetricsRegistry;
use prometheus::Registry;

const METRICS_PORT: u16 = 9091;
const TOTAL_SIZE: usize = 50 * 1024 * 1024 * 1024; // 1 GiB
const ENTRY_SIZE: usize = 7 * 1024 * 1024; // 7 MiB
const EXPECTED_ENTRIES: u32 = (TOTAL_SIZE / ENTRY_SIZE) as u32;
// on-disk size: 8B length prefix + ENTRY_SIZE value + 4B key + 36B header, page-aligned
const ALIGNED_ENTRY_SIZE: usize = ((ENTRY_SIZE + 8 + 4 + 36) + 4095) / 4096 * 4096;

enum Mode {
    Execute,
    Verify,
    ExecuteCustom,
    VerifyCustom,
}

fn parse_mode() -> anyhow::Result<Mode> {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--execute") => Ok(Mode::Execute),
        Some("--verify") => Ok(Mode::Verify),
        Some("--execute-custom") => Ok(Mode::ExecuteCustom),
        Some("--verify-custom") => Ok(Mode::VerifyCustom),
        other => {
            anyhow::bail!(
                "usage: data_cache_bench --execute | --verify | --execute-custom | --verify-custom (got {:?})",
                other
            )
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mode = parse_mode()?;

    match mode {
        Mode::Execute | Mode::Verify => {
            let registry = Registry::new();
            spawn_metrics_server(registry.clone());

            let flusher_count = num_cpus::get();
            let cache = init_foyer_cache(flusher_count, &registry).await?;

            match mode {
                Mode::Execute => execute(&cache).await?,
                Mode::Verify => verify(&cache).await?,
                _ => unreachable!(),
            }

            cache.close().await?;
        }
        Mode::ExecuteCustom => execute_custom().await?,
        Mode::VerifyCustom => verify_custom().await?,
    }

    Ok(())
}

async fn execute(cache: &HybridCache<u32, Vec<u8>>) -> anyhow::Result<()> {
    let mut data_buf = vec![0u8; ENTRY_SIZE];
    rand::fill(&mut data_buf[..]);

    let _props = HybridCacheProperties::default().with_location(Location::OnDisk);
    let start = Instant::now();
    let mut size_cnt: usize = 0;
    let mut cnt: u32 = 1;

    while size_cnt < TOTAL_SIZE {
        // cache.insert_with_properties(cnt, data_buf.clone(), props.clone());
        cache.insert(cnt, data_buf.clone());
        size_cnt += data_buf.len();
        cnt += 1;

        // if cnt % 10 == 0 {
        //     tokio::time::sleep(Duration::from_millis(30)).await;
        // }

        // if cnt % 3 == 0 {
        //     tokio::task::yield_now().await;
        // }
    }

    let elapsed = start.elapsed();
    let mib = (size_cnt as f64) / (1024.0 * 1024.0);

    println!(
        "execute: wrote {} entries ({:.1} MiB) in {:.2?} ({:.1} MiB/s)",
        cnt - 1,
        mib,
        elapsed,
        mib / elapsed.as_secs_f64()
    );

    Ok(())
}

async fn verify(cache: &HybridCache<u32, Vec<u8>>) -> anyhow::Result<()> {
    let start = Instant::now();
    let mut hits: u32 = 0;
    let mut misses: Vec<u32> = Vec::new();

    for key in 1..=EXPECTED_ENTRIES {
        match cache.get(&key).await? {
            Some(_) => hits += 1,
            None => misses.push(key),
        }
    }

    let elapsed = start.elapsed();
    let total = EXPECTED_ENTRIES;
    let miss_count = misses.len() as u32;
    let hit_rate = (hits as f64 / total as f64) * 100.0;
    let mib = (hits as f64 * ENTRY_SIZE as f64) / (1024.0 * 1024.0);

    println!("--- verification report ---");
    println!("keys checked:   {total}");
    println!("hits:           {hits}");
    println!("misses:         {miss_count}");
    println!("hit rate:       {hit_rate:.2}%");
    println!("elapsed:        {elapsed:.2?}");
    println!("read throughput: {:.1} MiB/s", mib / elapsed.as_secs_f64());

    if !misses.is_empty() {
        let preview: Vec<_> = misses.iter().take(10).collect();
        println!("first missing keys: {preview:?}");
    }

    Ok(())
}

async fn execute_custom() -> anyhow::Result<()> {
    tokio::fs::create_dir_all("./custom-data").await?;

    let mut data_buf = vec![0u8; ENTRY_SIZE];
    rand::fill(&mut data_buf[..]);

    let start = Instant::now();
    let mut size_cnt: usize = 0;
    let mut cnt: u32 = 1;

    while size_cnt < TOTAL_SIZE {
        let path = format!("./custom-data/{cnt}");
        tokio::fs::write(&path, &data_buf).await?;
        size_cnt += data_buf.len();
        cnt += 1;
    }

    let elapsed = start.elapsed();
    let mib = (size_cnt as f64) / (1024.0 * 1024.0);

    println!(
        "execute-custom: wrote {} files ({:.1} MiB) in {:.2?} ({:.1} MiB/s)",
        cnt - 1,
        mib,
        elapsed,
        mib / elapsed.as_secs_f64()
    );
    Ok(())
}

async fn verify_custom() -> anyhow::Result<()> {
    let start = Instant::now();
    let mut hits: u32 = 0;
    let mut misses: Vec<u32> = Vec::new();

    for key in 1..=EXPECTED_ENTRIES {
        let path = format!("./custom-data/{key}");
        match tokio::fs::metadata(&path).await {
            Ok(m) if m.len() == ENTRY_SIZE as u64 => hits += 1,
            _ => misses.push(key),
        }
    }

    let elapsed = start.elapsed();
    let total = EXPECTED_ENTRIES;
    let miss_count = misses.len() as u32;
    let hit_rate = (hits as f64 / total as f64) * 100.0;
    let mib = (hits as f64 * ENTRY_SIZE as f64) / (1024.0 * 1024.0);

    println!("--- custom verification report ---");
    println!("keys checked:    {total}");
    println!("hits:            {hits}");
    println!("misses:          {miss_count}");
    println!("hit rate:        {hit_rate:.2}%");
    println!("elapsed:         {elapsed:.2?}");
    println!("read throughput: {:.1} MiB/s", mib / elapsed.as_secs_f64());

    if !misses.is_empty() {
        let preview: Vec<_> = misses.iter().take(10).collect();
        println!("first missing keys: {preview:?}");
    }
    Ok(())
}

async fn init_foyer_cache(
    flusher_count: usize,
    registry: &Registry,
) -> anyhow::Result<HybridCache<u32, Vec<u8>>> {
    let builder = HybridCacheBuilder::new()
        .with_policy(HybridCachePolicy::WriteOnInsertion)
        .with_metrics_registry(Box::new(PrometheusMetricsRegistry::new(registry.clone())));

    // we want to keep memory to min
    let memory_phase = builder.memory(0).with_eviction_config(LfuConfig::default());

    let device = FsDeviceBuilder::new("./foyer-data")
        .with_capacity(TOTAL_SIZE * 2)
        .build()?;

    let storage_phase = memory_phase
        .storage()
        .with_recover_mode(RecoverMode::Strict)
        .with_engine_config(
            BlockEngineConfig::new(device)
                .with_flushers(flusher_count)
                .with_buffer_pool_size(ALIGNED_ENTRY_SIZE * flusher_count * 3)
                .with_submit_queue_size_threshold(ENTRY_SIZE * flusher_count * 2),
        );

    let cache = storage_phase.build().await?;

    Ok(cache)
}

fn spawn_metrics_server(registry: Registry) {
    tokio::spawn(async move {
        use hyper::{
            Body, Request, Response, Server, StatusCode,
            service::{make_service_fn, service_fn},
        };
        use prometheus::{Encoder, TextEncoder};

        let make_svc = make_service_fn(move |_| {
            let reg = registry.clone();
            async move {
                Ok::<_, std::convert::Infallible>(service_fn(move |req: Request<Body>| {
                    let reg = reg.clone();
                    async move {
                        let resp = match req.uri().path() {
                            "/metrics" => {
                                let encoder = TextEncoder::new();
                                let mut buf = Vec::new();
                                encoder.encode(&reg.gather(), &mut buf).unwrap();
                                Response::builder()
                                    .header(hyper::header::CONTENT_TYPE, encoder.format_type())
                                    .body(Body::from(buf))
                                    .unwrap()
                            }
                            _ => Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Body::from("not found"))
                                .unwrap(),
                        };
                        Ok::<_, std::convert::Infallible>(resp)
                    }
                }))
            }
        });

        let addr = ([0, 0, 0, 0], METRICS_PORT).into();
        if let Err(e) = Server::bind(&addr).serve(make_svc).await {
            eprintln!("metrics server error: {e}");
        }
    });
}

// When we were getting zero hits:
//
// Things to try:
// - Set entry size as block size
// - Change submit queue threshold to depend on 16MB size instead
// - Increase in-mem cache to hold just entry_size
// - Increase in-mem cache to hold entry_size * 2
// - Experiment with WriteOnInsert and WriteOnEviction more
// - Find a way to check if something is written in files
// - Do not use insert_with_props, if using in-mem cache with
// higher size
//
// Things already tried:
// - memory = 0 with WriteOnInsertion
// - memory = ENTRY_SIZE with WriteOnInsertion
// - memory = ENTRY_SIZE with WriteOnInsertion with no insert_with_props
// - memory = ENTRY_SIZE with WriteOnEviction
// - memory = ENTRY_SIZE with WriteOnEviction with no insert_with_props
//
// Real problem was buffer_pool_size was much smaller and everything was getting dropped
