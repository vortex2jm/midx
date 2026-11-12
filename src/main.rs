mod application;
mod cli;
mod config;
mod domain;
mod infrastructure;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;

use crate::application::ports::NoteRepository;
use crate::application::search::SearchUseCase;
use crate::application::sync::SyncUseCase;
use crate::cli::{Cli, Commands};
use crate::config::{
    DEFAULT_DB_REL_PATH, LoadedConfig, ensure_gitignore_entry, write_default_config_if_missing,
};
use crate::domain::models::SearchHit;
use crate::infrastructure::fs_scanner::FsScanner;
use crate::infrastructure::onnx_embedder::OnnxEmbedder;
use crate::infrastructure::sqlite_repo::SqliteStore;

fn main() {
    if let Err(err) = run() {
        eprintln!("{} {err:#}", "error:".red().bold());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => cmd_init(path),
        Commands::Sync { force } => cmd_sync(force),
        Commands::Search {
            query,
            limit,
            no_sync,
        } => cmd_search(&query, limit, no_sync),
        Commands::Status => cmd_status(),
    }
}

fn cmd_init(path: PathBuf) -> Result<()> {
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create directory '{}'", path.display()))?;
    let target_dir = fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve directory '{}'", path.display()))?;

    let (config_path, config_created) = write_default_config_if_missing(&target_dir)?;
    let db_path = target_dir.join(DEFAULT_DB_REL_PATH);
    let _store = SqliteStore::open(&db_path)?;
    let gitignore_updated = ensure_gitignore_entry(&target_dir)?;

    println!(
        "{} midx repository at {}",
        "Initialized".green().bold(),
        target_dir.display()
    );
    if config_created {
        println!("  {} {}", "created".cyan(), config_path.display());
    } else {
        println!(
            "  {} {} (kept existing)",
            "found".yellow(),
            config_path.display()
        );
    }
    println!("  {} {}", "database".cyan(), db_path.display());
    if gitignore_updated {
        println!("  {} .midx/ added to .gitignore", "updated".cyan());
    }

    Ok(())
}

fn cmd_sync(force: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to read current working directory")?;
    let loaded = LoadedConfig::discover(&cwd)?;

    let scanner = FsScanner::new();
    let repo = SqliteStore::open(&loaded.db_abs_path())?;
    let embedder = OnnxEmbedder::new(&loaded.config.model, true)?;

    let use_case = SyncUseCase::new(
        &scanner,
        &embedder,
        &repo,
        loaded.config.max_chunk_words,
        loaded.config.batch_size,
    );

    let summary = use_case.execute(&loaded.root_dir, force)?;

    println!("{}", "Sync completed".green().bold());
    println!("  added:     {}", summary.added.to_string().green());
    println!("  updated:   {}", summary.updated.to_string().yellow());
    println!("  removed:   {}", summary.removed.to_string().red());
    println!("  unchanged: {}", summary.unchanged);
    println!("  chunks:    {}", summary.chunks_indexed);
    println!("  elapsed:   {:.2}s", summary.elapsed.as_secs_f64());

    Ok(())
}

fn cmd_search(query: &str, limit: usize, no_sync: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to read current working directory")?;
    let loaded = LoadedConfig::discover(&cwd)?;

    let scanner = FsScanner::new();
    let repo = SqliteStore::open(&loaded.db_abs_path())?;
    let embedder = OnnxEmbedder::new(&loaded.config.model, true)?;

    let use_case = SearchUseCase::new(
        &scanner,
        &embedder,
        &repo,
        loaded.config.max_chunk_words,
        loaded.config.batch_size,
    );

    let hits = use_case.execute(&loaded.root_dir, query, limit, no_sync)?;
    if hits.is_empty() {
        println!("{}", "No matching notes found.".yellow());
        return Ok(());
    }

    for (idx, hit) in hits.iter().enumerate() {
        print_hit(idx + 1, hit);
    }

    Ok(())
}

fn cmd_status() -> Result<()> {
    let cwd = std::env::current_dir().context("failed to read current working directory")?;
    let loaded = LoadedConfig::discover(&cwd)?;
    let db_path = loaded.db_abs_path();
    let repo = SqliteStore::open(&db_path)?;
    let stats = repo.get_stats()?;

    println!("{}", "midx status".cyan().bold());
    println!("  root:          {}", loaded.root_dir.display());
    println!("  database:      {}", db_path.display());
    println!("  model:         {}", loaded.config.model.green());
    println!("  batch size:    {}", loaded.config.batch_size);
    println!("  indexed files: {}", stats.total_files);
    println!("  total chunks:  {}", stats.total_chunks);
    println!(
        "  db size:       {} ({} bytes)",
        format_bytes(stats.db_size_bytes),
        stats.db_size_bytes
    );

    Ok(())
}

fn print_hit(rank: usize, hit: &SearchHit) {
    let score_str = format!("{:.4}", hit.score);
    let location = format!("{}:{}", hit.file_path, hit.line_start);

    println!(
        "{} {} {}",
        format!("#{rank}").bold(),
        format!("[{score_str}]").green().bold(),
        location.cyan().bold()
    );

    if !hit.heading_context.is_empty() {
        println!("   {}", hit.heading_context.yellow());
    }

    let snippet = build_snippet(&hit.content, 240);
    for line in snippet.lines() {
        println!("   {}", line);
    }
    println!();
}

fn build_snippet(content: &str, max_chars: usize) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}
