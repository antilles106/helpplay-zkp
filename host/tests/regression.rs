//! Regression tests on real problems in `examples/`.
//!
//! Each case loads the problem and solution files the same way as the CLI, checks them with
//! `helpplay_core::verify`, and then runs the guest in the zkVM executor (no proof is made)
//! to confirm that the guest accepts them and commits the expected claim.
//!
//! Run with `cargo regression` (alias defined in .cargo/config.toml).

use helpplay_core::{verify, Input};
use host::{execute, load_problem, load_solutions};

struct Case {
    problem: &'static str,
    solutions: &'static str,
    label: &'static str,
    k: u32,
}

fn example(file: &str) -> String {
    format!("{}/../examples/{file}", env!("CARGO_MANIFEST_DIR"))
}

fn run(case: Case) {
    let problem = load_problem(&example(case.problem)).expect("load problem");
    let solutions = load_solutions(&example(case.solutions)).expect("load solutions");
    assert_eq!(problem.label(), case.label);
    assert_eq!(problem.k, case.k);

    if let Err(e) = verify(&problem, &solutions) {
        panic!("{}: solutions are not valid: {e}", case.problem);
    }

    let input = Input { problem: problem.clone(), claimant: "regression".into(), solutions };
    let claim = execute(&input).unwrap_or_else(|e| panic!("{}: guest failed: {e:?}", case.problem));
    assert_eq!(claim.problem, problem);
    assert_eq!(claim.claimant, "regression");
}

/// h#2, two solutions. FEN has only the piece placement (Black to move is filled in).
/// Solution 1 needs the disambiguated "N2d4".
#[test]
fn hmazemmour25() {
    run(Case {
        problem: "hmazemmour25.json",
        solutions: "hmazemmour25_sol.txt",
        label: "h#2",
        k: 2,
    });
}

/// h#4.5. FEN has only the piece placement (White to move is filled in).
#[test]
fn hmhopper25() {
    run(Case {
        problem: "hmhopper25.json",
        solutions: "hmhopper25_sol.txt",
        label: "h#4.5",
        k: 1,
    });
}

/// Release the position with a full FEN (side to move and castling rights are compared).
#[test]
fn release() {
    run(Case {
        problem: "release_problem.json",
        solutions: "release_solution.txt",
        label: "Release",
        k: 1,
    });
}
