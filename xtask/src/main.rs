//! Build tasks: `cargo xtask ci` runs what CI runs; `cargo xtask openapi`
//! writes the API description to `sdk/openapi.json`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("ci") => ci(),
        Some("openapi") => openapi(),
        _ => bail!("usage: cargo xtask <ci|openapi>"),
    }
}

/// Formatting, lints, tests and dependency checks, stopping at the first failure.
fn ci() -> anyhow::Result<()> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace", "--all-features"])?;
    cargo(&["deny", "check"])
}

fn openapi() -> anyhow::Result<()> {
    let path = root().join("sdk/openapi.json");
    let json = crust_api::openapi()
        .to_pretty_json()
        .context("serialising the OpenAPI description")?;
    std::fs::write(&path, json + "\n").with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn cargo(args: &[&str]) -> anyhow::Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!("> cargo {}", args.join(" "));
    let status = Command::new(cargo)
        .args(args)
        .current_dir(root())
        .status()
        .context("running cargo")?;
    if !status.success() {
        bail!("cargo {} failed", args.join(" "));
    }
    Ok(())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}
