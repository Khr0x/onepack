//! Carga de lectura de metadatos contra `onepackd` (Fase 7): varios clientes concurrentes
//! piden, en rueda, service index, flat container, registro y búsqueda durante un tiempo
//! fijo. Imprime latencias (p50/p95/p99) y throughput en JSON.
//!
//! Uso: `onepack-bench <url> <feed> <token> <paquete> [concurrencia] [segundos]`

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("uso: onepack-bench <url> <feed> <token> <paquete> [concurrencia] [segundos]");
        std::process::exit(2);
    }
    let (url, feed, token, package) = (&args[1], &args[2], &args[3], args[4].to_lowercase());
    let concurrency: usize = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(16);
    let seconds: u64 = args.get(6).and_then(|v| v.parse().ok()).unwrap_or(15);
    let base = format!("{}/nuget/{feed}/v3", url.trim_end_matches('/'));
    let paths = Arc::new(vec![
        ("service-index", format!("{base}/index.json")),
        ("flat", format!("{base}/flat/{package}/index.json")),
        (
            "registration",
            format!("{base}/registration/{package}/index.json"),
        ),
        ("search", format!("{base}/query?q=bench&take=20")),
    ]);

    // Calentamiento: una petición por recurso, y comprobación de que responden.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    for (name, path) in paths.iter() {
        let status = agent
            .get(path)
            .header("X-NuGet-ApiKey", token)
            .call()
            .map(|r| r.status().as_u16())
            .unwrap_or(0);
        if status != 200 {
            eprintln!("{name}: HTTP {status} en {path}");
            std::process::exit(1);
        }
    }

    let samples: Arc<Mutex<Vec<(usize, f64)>>> = Arc::new(Mutex::new(Vec::new()));
    let errors = Arc::new(Mutex::new(0u64));
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let started = Instant::now();
    let workers: Vec<_> = (0..concurrency)
        .map(|w| {
            let (paths, samples, errors) = (paths.clone(), samples.clone(), errors.clone());
            let token = token.clone();
            std::thread::spawn(move || {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(30)))
                    .build()
                    .into();
                let mut local = Vec::new();
                let mut i = w;
                while Instant::now() < deadline {
                    let which = i % paths.len();
                    let t = Instant::now();
                    let ok = agent
                        .get(&paths[which].1)
                        .header("X-NuGet-ApiKey", &token)
                        .call()
                        .and_then(|mut r| r.body_mut().read_to_vec().map(|_| r.status().as_u16()))
                        .is_ok_and(|s| s == 200);
                    if ok {
                        local.push((which, t.elapsed().as_secs_f64() * 1000.0));
                    } else {
                        *errors.lock().unwrap() += 1;
                    }
                    i += 1;
                }
                samples.lock().unwrap().extend(local);
            })
        })
        .collect();
    for w in workers {
        w.join().expect("hilo del benchmark");
    }
    let elapsed = started.elapsed().as_secs_f64();
    let samples = samples.lock().unwrap();
    let summarize = |filter: Option<usize>| {
        let mut ms: Vec<f64> = samples
            .iter()
            .filter(|(w, _)| filter.is_none_or(|f| f == *w))
            .map(|(_, v)| *v)
            .collect();
        ms.sort_by(|a, b| a.total_cmp(b));
        serde_json::json!({
            "requests": ms.len(),
            "p50_ms": percentile(&ms, 50.0),
            "p95_ms": percentile(&ms, 95.0),
            "p99_ms": percentile(&ms, 99.0),
            "max_ms": ms.last().copied().unwrap_or(0.0),
        })
    };
    let mut by_resource = serde_json::Map::new();
    for (i, (name, _)) in paths.iter().enumerate() {
        by_resource.insert((*name).to_owned(), summarize(Some(i)));
    }
    let report = serde_json::json!({
        "concurrency": concurrency,
        "seconds": elapsed,
        "requests_per_second": samples.len() as f64 / elapsed,
        "errors": *errors.lock().unwrap(),
        "all": summarize(None),
        "by_resource": by_resource,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serializable")
    );
}
