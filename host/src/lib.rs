//! Loading problem and solution files, and running the guest. Shared by the CLI and the tests.
//!
//! problem.json : {"kind": "helpmate", "fen": "...", "n": 2.5, "k": 1, "exclude": [["...", "..."]]}
//!                kind: "helpmate" (default), "proof_game" or "release".
//!                n: move count, written as x, x.0 or x.5 (a number or a string). Not used for release.
//!                   x and x.0 mean 2x plies; x.5 means 2x+1 plies.
//!                Helpmate: fen is the starting position. h#x is Black first, h#x.5 White first.
//!                   If fen has only the piece placement, the side to move is filled in.
//!                Proof game: fen is the target position (only piece placement is compared). White first.
//!                Release: a legal game of any length from the initial position reaching fen.
//!                   A full FEN also fixes side to move, castling rights and en passant.
//!                k: number of pairwise distinct solutions to prove knowledge of (default 1).
//!                kind, k and exclude are optional.
//!                Metadata (all optional, ignored by the proof and not written to the journal):
//!                author (a string or a list of strings), source, year (number or string), award, note.
//! solution.txt : one solution per line, moves separated by whitespace (UCI or SAN).
//!                Blank lines are ignored. This file is kept secret.

pub mod logging;

use anyhow::{bail, Context, Result};
use helpplay_core::{Claim, Input, Kind, Problem};
use methods::HELPPLAY_GUEST_ELF;
use risc0_zkvm::{default_executor, ExecutorEnv};
use serde::Deserialize;
use std::fs;

/// problem.json as written by users. Converted to `Problem` (which keeps `n` and `half` separately).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProblemFile {
    #[serde(default)]
    kind: Kind,
    fen: String,
    n: Option<serde_json::Value>,
    k: Option<u32>,
    #[serde(default)]
    exclude: Vec<Vec<String>>,
    // Metadata. Listed here rather than flattened, because serde's `flatten` does not work
    // together with `deny_unknown_fields`.
    author: Option<Authors>,
    source: Option<String>,
    year: Option<Year>,
    award: Option<String>,
    note: Option<String>,
}

/// Descriptive information about a problem. Not part of the proof.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub author: Option<Authors>,
    pub source: Option<String>,
    pub year: Option<Year>,
    pub award: Option<String>,
    pub note: Option<String>,
}

/// One author or several (joint compositions).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Authors {
    One(String),
    Many(Vec<String>),
}

/// A year written as a number (2025) or a string ("2025", "2024-25").
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Year {
    Number(u32),
    Text(String),
}

impl Meta {
    /// A one-line summary such as "A. Author & B. Author, Some Magazine 2025, 1st Prize",
    /// or None if no metadata is given. `note` is not included.
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        match &self.author {
            Some(Authors::One(a)) => parts.push(a.clone()),
            Some(Authors::Many(list)) if !list.is_empty() => parts.push(list.join(" & ")),
            _ => {}
        }
        let year = match &self.year {
            Some(Year::Number(y)) => Some(y.to_string()),
            Some(Year::Text(y)) => Some(y.clone()),
            None => None,
        };
        match (&self.source, year) {
            (Some(s), Some(y)) => parts.push(format!("{s} {y}")),
            (Some(s), None) => parts.push(s.clone()),
            (None, Some(y)) => parts.push(y),
            (None, None) => {}
        }
        if let Some(a) = &self.award {
            parts.push(a.clone());
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

/// Parses a move count written as x, x.0 or x.5 into (x, is_half).
pub fn parse_count(v: &serde_json::Value) -> Result<(u32, bool)> {
    let s = match v {
        serde_json::Value::Number(num) => num.to_string(),
        serde_json::Value::String(s) => s.trim().to_string(),
        _ => bail!("n must be a number or a string such as 2, 2.0 or 2.5"),
    };
    let (int, frac) = match s.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (s.as_str(), None),
    };
    let half = match frac {
        None | Some("0") => false,
        Some("5") => true,
        _ => bail!("n must be written as x, x.0 or x.5 (got {s})"),
    };
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        bail!("n must be written as x, x.0 or x.5 (got {s})");
    }
    let n = int.parse().with_context(|| format!("n is too large: {s}"))?;
    Ok((n, half))
}

/// Parses the contents of a problem.json file into the problem and its metadata.
pub fn parse_problem(json: &str) -> Result<(Problem, Meta)> {
    let file: ProblemFile = serde_json::from_str(json)?;
    let (n, half) = match (file.kind, &file.n) {
        (Kind::Release, None) => (0, false),
        (Kind::Release, Some(_)) => bail!("n is not used for release (the game may have any length)"),
        (_, Some(n)) => parse_count(n)?,
        (_, None) => bail!("missing field `n`"),
    };
    let problem = Problem {
        kind: file.kind,
        fen: file.fen,
        n,
        half,
        exclude: file.exclude,
        k: file.k.unwrap_or(1),
    };
    let meta = Meta {
        author: file.author,
        source: file.source,
        year: file.year,
        award: file.award,
        note: file.note,
    };
    Ok((problem, meta))
}

/// Reads a problem.json file, returning the problem and its metadata.
pub fn load_problem_with_meta(path: &str) -> Result<(Problem, Meta)> {
    let s = fs::read_to_string(path).with_context(|| format!("cannot read {path}"))?;
    parse_problem(&s).with_context(|| format!("invalid problem file {path}"))
}

/// Reads a problem.json file. Metadata is ignored.
pub fn load_problem(path: &str) -> Result<Problem> {
    Ok(load_problem_with_meta(path)?.0)
}

/// Reads a solution file: one solution per line, blank lines ignored.
pub fn load_solutions(path: &str) -> Result<Vec<Vec<String>>> {
    Ok(fs::read_to_string(path)
        .with_context(|| format!("cannot read {path}"))?
        .lines()
        .map(|line| line.split_whitespace().map(String::from).collect::<Vec<_>>())
        .filter(|moves| !moves.is_empty())
        .collect())
}

/// Runs the guest in the zkVM executor without proving, and returns the claim it commits.
/// Fails if the guest rejects the solutions.
pub fn execute(input: &Input) -> Result<Claim> {
    let env = ExecutorEnv::builder().write(input)?.build()?;
    let session =
        logging::timed("execute guest", || default_executor().execute(env, HELPPLAY_GUEST_ELF))?;
    Ok(session.journal.decode()?)
}

#[cfg(test)]
mod tests {
    use super::{parse_count, parse_problem, Authors, Year};
    use serde_json::json;

    const FEN: &str = "7k/8/8/6K1/8/8/8/R7";

    #[test]
    fn metadata_is_read_but_not_part_of_the_problem() {
        let plain = format!(r#"{{"fen": "{FEN}", "n": 2}}"#);
        let with_meta = format!(
            r#"{{"fen": "{FEN}", "n": 2, "author": ["A. One", "B. Two"],
                "source": "Some Magazine", "year": 2025, "award": "1st Prize", "note": "test"}}"#
        );
        let (p1, m1) = parse_problem(&plain).unwrap();
        let (p2, m2) = parse_problem(&with_meta).unwrap();
        // The proven problem is the same, so receipts are unaffected by metadata
        assert_eq!(p1, p2);
        assert_eq!(m1.summary(), None);
        assert_eq!(
            m2.author,
            Some(Authors::Many(vec!["A. One".into(), "B. Two".into()]))
        );
        assert_eq!(m2.note.as_deref(), Some("test"));
        assert_eq!(
            m2.summary().as_deref(),
            Some("A. One & B. Two, Some Magazine 2025, 1st Prize")
        );
    }

    #[test]
    fn metadata_accepts_single_author_and_text_year() {
        let json = format!(r#"{{"fen": "{FEN}", "n": 2, "author": "A. One", "year": "2024-25"}}"#);
        let (_, m) = parse_problem(&json).unwrap();
        assert_eq!(m.author, Some(Authors::One("A. One".into())));
        assert_eq!(m.year, Some(Year::Text("2024-25".into())));
        assert_eq!(m.summary().as_deref(), Some("A. One, 2024-25"));
    }

    #[test]
    fn n_is_required_except_for_release() {
        let (p, _) = parse_problem(&format!(r#"{{"kind": "release", "fen": "{FEN}"}}"#)).unwrap();
        assert_eq!((p.n, p.half), (0, false));
        assert!(parse_problem(&format!(r#"{{"kind": "release", "fen": "{FEN}", "n": 2}}"#)).is_err());
        assert!(parse_problem(&format!(r#"{{"fen": "{FEN}"}}"#)).is_err());
        assert!(parse_problem(&format!(r#"{{"kind": "proof_game", "fen": "{FEN}"}}"#)).is_err());
    }

    #[test]
    fn unknown_keys_are_still_rejected() {
        let json = format!(r#"{{"fen": "{FEN}", "n": 2, "autor": "typo"}}"#);
        assert!(parse_problem(&json).is_err());
    }

    #[test]
    fn accepts_x_x0_x5_as_numbers_and_strings() {
        assert_eq!(parse_count(&json!(2)).unwrap(), (2, false));
        assert_eq!(parse_count(&json!(2.0)).unwrap(), (2, false));
        assert_eq!(parse_count(&json!(2.5)).unwrap(), (2, true));
        assert_eq!(parse_count(&json!(0.5)).unwrap(), (0, true));
        assert_eq!(parse_count(&json!("3")).unwrap(), (3, false));
        assert_eq!(parse_count(&json!("3.0")).unwrap(), (3, false));
        assert_eq!(parse_count(&json!("3.5")).unwrap(), (3, true));
    }

    #[test]
    fn rejects_other_values() {
        for v in [json!(2.25), json!(-1), json!(-0.5), json!("2.3"), json!("x"), json!(".5"),
                  json!("2."), json!(true), json!(null)] {
            assert!(parse_count(&v).is_err(), "{v}");
        }
    }
}
