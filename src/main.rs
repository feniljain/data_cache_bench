use foyer::{
    BlockEngineConfig, DeviceBuilder, FsDeviceBuilder, HybridCache, HybridCacheBuilder,
    HybridCacheProperties, LfuConfig, Location,
};
use prometheus::Registry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut data_buf = vec![0; 920000];

    let total_size = 100_000_000_000; // 100GB
    let mut size_cnt = 0;
    let mut cnt = 1;

    let flusher_count = num_cpus::get();
    let entry_size = 7360000; // 7 MiB
    let cache = init_foyer_cache(total_size, flusher_count, entry_size).await?;
    let foyer_cache_props = HybridCacheProperties::default().with_location(Location::OnDisk);

    loop {
        if size_cnt > total_size {
            break;
        }

        rand::fill(&mut data_buf[..]);

        // write to cache
        //
        // foyer
        cache.insert_with_properties(cnt, data_buf.clone(), foyer_cache_props.clone());

        //
        // custom

        size_cnt += 920000;
        cnt += 1;
    }

    Ok(())
}

async fn init_foyer_cache(
    total_size: usize,
    flusher_count: usize,
    entry_size: usize,
) -> anyhow::Result<HybridCache<u32, Vec<u8>>> {
    let builder = HybridCacheBuilder::new();

    // TODO: expose foyer metrics with foyer prefix to local prom server
    // accessible on localhost:9090
    // if let Some(registry) = metrics_registry {
    //     builder = builder.with_metrics_registry(registry);
    // }

    // We store nothing in memory
    let memory_phase = builder.memory(0).with_eviction_config(LfuConfig::default());

    let mut storage_phase = memory_phase.storage();

    let device = FsDeviceBuilder::new("./foyer-data")
        .with_capacity(total_size)
        .build()?;

    storage_phase = storage_phase.with_engine_config(
        BlockEngineConfig::new(device)
            .with_flushers(flusher_count)
            .with_submit_queue_size_threshold(entry_size * flusher_count * 2),
    );

    let cache = storage_phase.build().await?;

    Ok(cache)
}
