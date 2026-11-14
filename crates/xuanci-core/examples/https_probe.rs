use std::time::{Duration, Instant};

fn main() {
    let config = ureq::config::Config::builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_global(Some(Duration::from_secs(15)))
        .build();
    let agent = config.new_agent();

    let started = Instant::now();
    match agent.get("https://api.deepseek.com/").call() {
        Ok(response) => println!(
            "ok status={} elapsed={:?}",
            response.status(),
            started.elapsed()
        ),
        Err(error) => println!("error elapsed={:?}: {error}", started.elapsed()),
    }
}
