//! Dependency licence allowlist audit.

use std::collections::HashSet;
use std::process::Command;

use serde_json::Value;

const ALLOWED: &[&str] = &[
    "0bsd",
    "apache-2.0",
    "bsd-2-clause",
    "bsd-3-clause",
    "bsl-1.0",
    "bzip2-1.0.6",
    "cc0-1.0",
    "isc",
    "llvm-exception",
    "mit",
    "mit-0",
    "mpl-2.0",
    "ofl-1.1",
    "ubuntu-font-1.0",
    "unicode-3.0",
    "unlicense",
    "zlib",
];

fn normalize(expression: &str) -> String {
    expression.replace('/', " OR ").trim().to_string()
}

fn split_top_level(expression: &str, operator: &str) -> Vec<String> {
    let needle = format!(" {operator} ");
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut chars = expression.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && expression[index..].starts_with(&needle) {
            parts.push(current.trim().to_string());
            current.clear();
            for _ in 0..needle.len() - 1 {
                chars.next();
            }
            continue;
        }
        current.push(ch);
    }
    parts.push(current.trim().to_string());
    parts.retain(|part| !part.is_empty());
    parts
}

fn strip_parens(expression: &str) -> String {
    let mut value = expression.trim();
    loop {
        if !(value.starts_with('(') && value.ends_with(')')) {
            return value.to_string();
        }
        let mut depth = 0i32;
        let mut balanced = true;
        for (index, ch) in value.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && index != value.len() - 1 {
                        balanced = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !balanced {
            return value.to_string();
        }
        value = value[1..value.len() - 1].trim();
    }
}

fn allowed(expression: &str) -> bool {
    let expression = strip_parens(&normalize(expression));
    let ors = split_top_level(&expression, "OR");
    if ors.len() > 1 {
        return ors.iter().any(|part| allowed(part));
    }
    let ands = split_top_level(&expression, "AND");
    if ands.len() > 1 {
        return ands.iter().all(|part| allowed(part));
    }
    if let Some((base, exception)) = expression.split_once(" WITH ") {
        return allowed(base) && ALLOWED.contains(&exception.trim().to_lowercase().as_str());
    }
    ALLOWED.contains(&expression.trim().to_lowercase().as_str())
}

pub fn run(_args: &[String]) -> Result<(), String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .output()
        .map_err(|err| format!("cargo metadata: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("parse cargo metadata: {err}"))?;
    let workspace: HashSet<&str> = metadata["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?;
    let mut seen = HashSet::new();
    let mut rejected = Vec::new();
    let mut missing = Vec::new();
    for package in packages {
        let Some(id) = package["id"].as_str() else {
            continue;
        };
        if workspace.contains(id) || !seen.insert(id.to_string()) {
            continue;
        }
        let name = package["name"].as_str().unwrap_or("?");
        let version = package["version"].as_str().unwrap_or("?");
        match package["license"].as_str() {
            Some(expression) if !expression.is_empty() => {
                if !allowed(expression) {
                    rejected.push(format!("{name} {version}: {expression}"));
                }
            }
            _ if package["license_file"].as_str().is_some() => {}
            _ => missing.push(format!("{name} {version}")),
        }
    }
    if !missing.is_empty() {
        println!("dependencies without a license field:");
        for item in &missing {
            println!("  {item}");
        }
    }
    if !rejected.is_empty() {
        println!("dependencies outside the allowlist:");
        for item in &rejected {
            println!("  {item}");
        }
    }
    if !missing.is_empty() || !rejected.is_empty() {
        return Err("license audit failed".into());
    }
    println!(
        "license audit: {} dependencies, all licenses allowed",
        seen.len()
    );
    Ok(())
}
