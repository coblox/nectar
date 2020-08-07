use log::LevelFilter;
use tracing::{info, subscriber};
use tracing_log::LogTracer;
use tracing_subscriber::FmtSubscriber;

pub fn init_tracing(level: log::LevelFilter) -> anyhow::Result<()> {
    if level == LevelFilter::Off {
        return Ok(());
    }

    LogTracer::init()?;

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(format!("nectar={},comit={},http=info,warp=info,hyper=info,reqwest=info,want=info,libp2p_gossipsub=info,sled=info,libp2p_core={}", level, level, level))
        .finish();

    subscriber::set_global_default(subscriber)?;
    info!("Initialized tracing with level: {}", level);

    Ok(())
}
