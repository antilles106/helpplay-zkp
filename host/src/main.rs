//! Usage:
//!   host [options] prove  <problem.json> <solution.txt> <claimant> <receipt.bin>
//!   host [options] verify <problem.json> <receipt.bin>
//!
//! --log <dest>        : where messages go: stdout (default), stderr, none, or a file path (appended).
//! --log-level <level> : debug, info (default), warn or error. debug adds timings of each step.
//!
//! See lib.rs for the file formats.

use anyhow::{bail, Context, Result};
use helpplay_core::{verify, Claim, Input, Kind};
use host::logging::{format_duration, handler_for, set_handler, set_level, timed, Level};
use host::{load_problem, load_problem_with_meta, load_solutions, log_debug, log_error, log_info};
use methods::{HELPPLAY_GUEST_ELF, HELPPLAY_GUEST_ID};
use risc0_zkvm::{default_prover, ExecutorEnv, InnerReceipt, ProverOpts, Receipt};
use std::fs;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage:\n  \
    host [options] prove  <problem.json> <solution.txt> <claimant> <receipt.bin>\n  \
    host [options] verify <problem.json> <receipt.bin>\n\
    options:\n  \
    --log <dest>         stdout (default), stderr, none, or a file path\n  \
    --log-level <level>  debug, info (default), warn or error";

fn prove(problem_path: &str, solution_path: &str, claimant: &str, out: &str) -> Result<()> {
    let start = Instant::now();
    let problem = timed("load problem", || load_problem(problem_path))?;
    let solutions = timed("load solutions", || load_solutions(solution_path))?;

    // Check locally before proving (catches mistakes before running the zkVM)
    if let Err(e) = timed("check solutions", || verify(&problem, &solutions)) {
        bail!("solutions are not valid: {e}");
    }

    let input = Input { problem, claimant: claimant.to_string(), solutions };
    let env = ExecutorEnv::builder().write(&input)?.build()?;
    // Output in Groth16 format (a small, constant-size proof suited to on-chain verification)
    let info = timed("prove (zkVM + Groth16)", || {
        default_prover().prove_with_opts(env, HELPPLAY_GUEST_ELF, &ProverOpts::groth16())
    })?;
    log_debug!(
        "stats: {} segment(s), {} total cycles ({} user, {} paging, {} reserved)",
        info.stats.segments,
        info.stats.total_cycles,
        info.stats.user_cycles,
        info.stats.paging_cycles,
        info.stats.reserved_cycles
    );

    timed("write receipt", || -> Result<()> {
        fs::write(out, bincode::serialize(&info.receipt)?)?;
        Ok(())
    })?;
    log_info!("receipt written to {out}");
    log_debug!("time: total: {}", format_duration(start.elapsed()));
    Ok(())
}

fn check(problem_path: &str, receipt_path: &str) -> Result<()> {
    let start = Instant::now();
    let (problem, meta) = timed("load problem", || load_problem_with_meta(problem_path))?;
    let receipt: Receipt = timed("load receipt", || -> Result<Receipt> {
        Ok(bincode::deserialize(&fs::read(receipt_path)?)?)
    })?;

    // 1. Was the proof produced by the right guest program (ImageID)?
    timed("verify receipt", || receipt.verify(HELPPLAY_GUEST_ID))
        .context("receipt verification failed")?;

    // 2. Is the proven problem the same as ours?
    let claim: Claim = receipt.journal.decode()?;
    if claim.problem != problem {
        bail!("receipt is for a different problem: {:?}", claim.problem);
    }

    let kind = match &receipt.inner {
        InnerReceipt::Groth16(_) => "Groth16",
        InnerReceipt::Succinct(_) => "Succinct",
        InnerReceipt::Composite(_) => "Composite",
        InnerReceipt::Fake(_) => "Fake (dev mode)",
        _ => "unknown",
    };

    let what = match (problem.kind, problem.k) {
        (Kind::Release, 1) => "a legal game reaching the position".to_string(),
        (Kind::Release, k) => format!("{k} distinct legal games reaching the position"),
        (_, 1) => format!("a solution of {}", problem.label()),
        (_, k) => format!("{k} distinct solutions of {}", problem.label()),
    };
    log_info!("OK: {} knows {what}", claim.claimant);
    if let Some(summary) = meta.summary() {
        log_info!("    problem: {summary}");
    }
    log_info!("    receipt format: {kind}");
    if !problem.exclude.is_empty() {
        log_info!("    (different from the {} excluded solution(s))", problem.exclude.len());
    }
    log_debug!("time: total: {}", format_duration(start.elapsed()));
    Ok(())
}

/// Removes `<name> <value>` / `<name>=<value>` from the arguments and returns the value.
fn take_option(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let prefix = format!("{name}=");
    let Some(i) = args.iter().position(|a| a == name || a.starts_with(&prefix)) else {
        return Ok(None);
    };
    let arg = args.remove(i);
    match arg.strip_prefix(&prefix) {
        Some(value) => Ok(Some(value.to_string())),
        None if i < args.len() => Ok(Some(args.remove(i))),
        None => bail!("{name} needs a value\n{USAGE}"),
    }
}

fn run(mut args: Vec<String>) -> Result<()> {
    if let Some(level) = take_option(&mut args, "--log-level")? {
        set_level(level.parse::<Level>().map_err(anyhow::Error::msg)?);
    }
    if let Some(dest) = take_option(&mut args, "--log")? {
        let handler = handler_for(&dest).with_context(|| format!("cannot open log {dest}"))?;
        set_handler(handler);
    }
    match args.get(1).map(String::as_str) {
        Some("prove") if args.len() == 6 => prove(&args[2], &args[3], &args[4], &args[5]),
        Some("verify") if args.len() == 4 => check(&args[2], &args[3]),
        _ => bail!("{USAGE}"),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `{:#}` puts the whole error chain on one line ("outer: inner: ...").
            log_error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::take_option;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn log_option_is_removed_from_anywhere() {
        let mut a = args("host --log stderr verify p.json r.bin");
        assert_eq!(take_option(&mut a, "--log").unwrap().as_deref(), Some("stderr"));
        assert_eq!(a, args("host verify p.json r.bin"));

        let mut a = args("host verify p.json r.bin --log=out.log");
        assert_eq!(take_option(&mut a, "--log").unwrap().as_deref(), Some("out.log"));
        assert_eq!(a, args("host verify p.json r.bin"));

        let mut a = args("host verify p.json r.bin");
        assert_eq!(take_option(&mut a, "--log").unwrap(), None);
    }

    #[test]
    fn log_and_log_level_are_told_apart() {
        let mut a = args("host --log-level debug --log run.log verify p.json r.bin");
        assert_eq!(take_option(&mut a, "--log-level").unwrap().as_deref(), Some("debug"));
        assert_eq!(take_option(&mut a, "--log").unwrap().as_deref(), Some("run.log"));
        assert_eq!(a, args("host verify p.json r.bin"));

        // "--log" must not match "--log-level=..."
        let mut a = args("host --log-level=debug verify p.json r.bin");
        assert_eq!(take_option(&mut a, "--log").unwrap(), None);
    }

    #[test]
    fn option_without_value_is_an_error() {
        assert!(take_option(&mut args("host verify p.json r.bin --log"), "--log").is_err());
    }
}
