//! OSV.dev advisory gate for the locked dependency set.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

const BASELINE: &str = "xtask/osv-baseline.txt";

fn baseline() -> Result<HashSet<String>, String> {
    let text = fs::read_to_string(BASELINE).map_err(|err| format!("{BASELINE}: {err}"))?;
    Ok(text
        .lines()
        .filter_map(|line| line.split('#').next())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn run(_args: &[String]) -> Result<(), String> {
    let lock = fs::read_to_string("Cargo.lock").map_err(|err| format!("Cargo.lock: {err}"))?;
    let lock: toml::Value = toml::from_str(&lock).map_err(|err| format!("Cargo.lock: {err}"))?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or("Cargo.lock has no package array")?;
    let mut queries = Vec::new();
    for package in packages {
        let source = package
            .get("source")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        if !source.contains("crates.io") {
            continue;
        }
        queries.push(json!({
            "package": {
                "name": package.get("name").and_then(toml::Value::as_str).unwrap_or_default(),
                "ecosystem": "crates.io",
            },
            "version": package.get("version").and_then(toml::Value::as_str).unwrap_or_default(),
        }));
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|err| format!("HTTP client: {err}"))?;
    let response = client
        .post("https://api.osv.dev/v1/querybatch")
        .json(&json!({ "queries": queries }))
        .send()
        .map_err(|err| format!("OSV request: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("OSV returned {}", response.status()));
    }
    let result: Value = response.json().map_err(|err| format!("OSV JSON: {err}"))?;
    let accepted = baseline()?;
    let mut findings = Vec::new();
    for (query, entry) in queries
        .iter()
        .zip(result["results"].as_array().into_iter().flatten())
    {
        for advisory in entry["vulns"].as_array().into_iter().flatten() {
            let id = advisory["id"].as_str().unwrap_or("unknown");
            findings.push((
                id.to_string(),
                query["package"]["name"].as_str().unwrap_or("?").to_string(),
                query["version"].as_str().unwrap_or("?").to_string(),
                advisory["summary"].as_str().unwrap_or("").to_string(),
            ));
        }
    }
    let new: Vec<_> = findings
        .iter()
        .filter(|finding| !accepted.contains(&finding.0))
        .collect();
    println!(
        "OSV audit: {} crates.io packages, {} advisory match(es), {} new",
        queries.len(),
        findings.len(),
        new.len()
    );
    for (id, name, version, summary) in &findings {
        let marker = if accepted.contains(id) {
            "known"
        } else {
            "NEW"
        };
        println!("  {marker:5} {id}  {name} {version}  {summary}");
    }
    if !new.is_empty() {
        return Err("new advisories require a code/dependency fix or baseline review".into());
    }
    Ok(())
}

/// Keeps the relative path helper available for future baseline overrides.
#[allow(dead_code)]
fn baseline_path() -> &'static Path {
    Path::new(BASELINE)
}
