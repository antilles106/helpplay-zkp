//! Logic for verifying solutions of helpmates (h#n and h#n.5), proof games (PG n.0 and PG n.5)
//! and Release the position (a legal game of any length reaching a position).
//! The same code is used by both the zkVM guest and the host.

use serde::{Deserialize, Serialize};
use shakmaty::{
    fen::Fen,
    san::{San, SanError},
    uci::UciMove,
    Board, CastlingMode, Chess, Color, EnPassantMode, Move, Position, Setup,
};

/// The kind of problem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Helpmate. `fen` is the starting position.
    #[default]
    Helpmate,
    /// Proof game. Play starts from the standard initial position and reaches the placement in `fen`.
    ProofGame,
    /// Release the position: a legal game of any length from the standard initial position that
    /// reaches the position in `fen`, showing that the position is legal. `n` and `half` are unused.
    Release,
}

/// The public problem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// The kind of problem. Defaults to helpmate.
    #[serde(default)]
    pub kind: Kind,
    /// Helpmate: the starting position (FEN). Black must be to move for h#n, White for h#n.5.
    /// If only the piece placement is given, the side to move follows from the form
    /// (no castling rights, no en passant).
    /// Proof game: the target position. Only the piece placement (first FEN field) is compared.
    /// The side to move follows from the move count; castling rights and en passant are not compared.
    /// Release: the target position. With only the piece placement, only the placement is compared.
    /// With a full FEN, the side to move, castling rights and en passant square are compared too
    /// (the en passant square must be given only when an en passant capture is legal).
    pub fen: String,
    /// The integer part n of the move count.
    pub n: u32,
    /// Helpmate: true for h#n.5 (White first, 2n+1 plies), false for h#n (Black first, 2n plies).
    /// Proof game: true for PG n.5 (2n+1 plies), false for PG n.0 (2n plies). White moves first in both.
    #[serde(default)]
    pub half: bool,
    /// Known solutions (e.g. the author's). If given, the proof shows "a different solution" (for cooks).
    #[serde(default)]
    pub exclude: Vec<Vec<String>>,
    /// Number of pairwise distinct solutions the prover claims to know. Defaults to 1.
    #[serde(default = "one")]
    pub k: u32,
}

fn one() -> u32 {
    1
}

impl Problem {
    /// Length of the solution in plies.
    pub fn plies(&self) -> usize {
        2 * self.n as usize + self.half as usize
    }

    /// The side that moves first.
    pub fn first_mover(&self) -> Color {
        match self.kind {
            Kind::Helpmate if !self.half => Color::Black,
            _ => Color::White,
        }
    }

    /// A label such as "h#2", "h#2.5", "PG 4.0", "PG 3.5" or "Release".
    pub fn label(&self) -> String {
        match (self.kind, self.half) {
            (Kind::Release, _) => "Release".to_string(),
            (Kind::Helpmate, false) => format!("h#{}", self.n),
            (Kind::Helpmate, true) => format!("h#{}.5", self.n),
            (Kind::ProofGame, false) => format!("PG {}.0", self.n),
            (Kind::ProofGame, true) => format!("PG {}.5", self.n),
        }
    }

    /// The position the moves are played from.
    fn start(&self) -> Result<Chess, Error> {
        match self.kind {
            Kind::Helpmate => {
                let fen: Fen = if self.fen.split_whitespace().count() == 1 {
                    // Placement only: fill in the side to move required by the form.
                    let side = if self.first_mover() == Color::White { 'w' } else { 'b' };
                    format!("{} {side} - - 0 1", self.fen.trim()).parse()
                } else {
                    self.fen.parse()
                }
                .map_err(|_| Error::BadFen)?;
                fen.into_position(CastlingMode::Standard).map_err(|_| Error::BadFen)
            }
            Kind::ProofGame | Kind::Release => Ok(Chess::default()),
        }
    }
}

/// What the final position of a proof game or Release must match.
enum Target {
    /// Only the piece placement.
    Placement(Board),
    /// Placement, side to move, castling rights and (legal) en passant square.
    Full(Setup),
}

impl Target {
    /// Release: a placement-only FEN gives `Placement`, a full FEN gives `Full`.
    fn release(fen: &str) -> Result<Target, Error> {
        if fen.split_whitespace().count() <= 1 {
            return Ok(Target::Placement(target_board(fen)?));
        }
        let fen: Fen = fen.parse().map_err(|_| Error::BadFen)?;
        Ok(Target::Full(fen.into_setup()))
    }

    fn matches(&self, end: &Chess) -> bool {
        match self {
            Target::Placement(board) => end.board() == board,
            Target::Full(t) => {
                let e = end.to_setup(EnPassantMode::Legal);
                e.board == t.board
                    && e.turn == t.turn
                    && e.castling_rights == t.castling_rights
                    && e.ep_square == t.ep_square
            }
        }
    }
}

/// Input to the guest. `solutions` is secret (never written to the journal).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub problem: Problem,
    /// Who the proof belongs to (a name or address). Included in the journal to stop others claiming the proof.
    pub claimant: String,
    /// `problem.k` solutions, each a list of moves.
    pub solutions: Vec<Vec<String>>,
}

/// Public information written to the journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub problem: Problem,
    pub claimant: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    BadFen,
    WrongSideToMove { expected: Color },
    WrongLength { expected: usize, got: usize },
    IllegalMove { ply: usize, mv: String },
    /// SAN that matches several legal moves. `candidates` are the disambiguated forms.
    AmbiguousMove { ply: usize, mv: String, candidates: Vec<String> },
    NotCheckmate,
    NotReached,
    SameAsExcluded,
    ZeroK,
    WrongCount { expected: usize, got: usize },
    /// An error in one solution (`index` is 1-based).
    InSolution { index: usize, error: Box<Error> },
    /// Two solutions are the same sequence (1-based indices).
    Duplicate { first: usize, second: usize },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::BadFen => write!(f, "cannot parse the FEN"),
            Error::WrongSideToMove { expected } => {
                let side = if *expected == Color::White { "White" } else { "Black" };
                write!(f, "{side} must be to move in the starting position for this form")
            }
            Error::WrongLength { expected, got } => {
                write!(f, "wrong number of moves ({expected} plies required, got {got})")
            }
            Error::IllegalMove { ply, mv } => write!(f, "ply {ply} ({mv}) is illegal"),
            Error::AmbiguousMove { ply, mv, candidates } => write!(
                f,
                "ply {ply} ({mv}) is ambiguous; write one of: {}",
                candidates.join(", ")
            ),
            Error::NotCheckmate => write!(f, "Black is not checkmated in the final position"),
            Error::NotReached => write!(
                f,
                "the final position does not match the target position \
                 (piece placement, and for a full FEN in Release also side to move, castling rights and en passant)"
            ),
            Error::SameAsExcluded => write!(f, "the moves are the same as an excluded known solution"),
            Error::ZeroK => write!(f, "k must be at least 1"),
            Error::WrongCount { expected, got } => {
                write!(f, "wrong number of solutions ({expected} required, got {got})")
            }
            Error::InSolution { index, error } => write!(f, "solution {index}: {error}"),
            Error::Duplicate { first, second } => {
                write!(f, "solutions {first} and {second} are the same sequence of moves")
            }
        }
    }
}

/// Piece placement of a proof game's target position. Accepts a full FEN or just the placement field.
fn target_board(fen: &str) -> Result<Board, Error> {
    let placement = fen.split_whitespace().next().ok_or(Error::BadFen)?;
    placement.parse().map_err(|_| Error::BadFen)
}

/// Accepts both UCI (e.g. "e2e4") and SAN (e.g. "Qh5#"). `ply` is only used in errors.
fn parse_move(pos: &Chess, s: &str, ply: usize) -> Result<Move, Error> {
    if let Ok(uci) = s.parse::<UciMove>() {
        if let Ok(m) = uci.to_move(pos) {
            return Ok(m);
        }
    }
    let illegal = || Error::IllegalMove { ply, mv: s.to_string() };
    let san: San = s.trim_end_matches(['+', '#', '!', '?']).parse().map_err(|_| illegal())?;
    match san.clone().to_move(pos) {
        Ok(m) => Ok(m),
        Err(SanError::AmbiguousSan) => {
            let candidates = pos
                .legal_moves()
                .iter()
                .filter(|m| san.clone().matches((*m).clone()))
                .map(|m| San::from_move(pos, m.clone()).to_string())
                .collect();
            Err(Error::AmbiguousMove { ply, mv: s.to_string(), candidates })
        }
        Err(SanError::IllegalSan) => Err(illegal()),
    }
}

/// Plays through the moves and returns each one normalized to UCI.
fn replay(problem: &Problem, moves: &[String]) -> Result<(Chess, Vec<String>), Error> {
    let mut pos = problem.start()?;
    let mut ucis = Vec::with_capacity(moves.len());
    for (i, s) in moves.iter().enumerate() {
        let m = parse_move(&pos, s, i + 1)?;
        ucis.push(m.to_uci(CastlingMode::Standard).to_string());
        pos.play_unchecked(m); // safe: parse_move only returns legal moves
    }
    Ok((pos, ucis))
}

/// Checks that `solutions` are `problem.k` correct and pairwise distinct solutions of `problem`.
pub fn verify(problem: &Problem, solutions: &[Vec<String>]) -> Result<(), Error> {
    let k = problem.k as usize;
    if k == 0 {
        return Err(Error::ZeroK);
    }
    if solutions.len() != k {
        return Err(Error::WrongCount { expected: k, got: solutions.len() });
    }

    let mut seen: Vec<Vec<String>> = Vec::with_capacity(k);
    for (i, moves) in solutions.iter().enumerate() {
        let ucis = verify_solution(problem, moves)
            .map_err(|e| Error::InSolution { index: i + 1, error: Box::new(e) })?;
        // Compare normalized moves, so the same sequence in other notation counts as the same.
        if let Some(j) = seen.iter().position(|s| *s == ucis) {
            return Err(Error::Duplicate { first: j + 1, second: i + 1 });
        }
        seen.push(ucis);
    }
    Ok(())
}

/// Checks whether `moves` is a correct solution of `problem` (ignoring `k`).
/// Returns the moves normalized to UCI.
pub fn verify_solution(problem: &Problem, moves: &[String]) -> Result<Vec<String>, Error> {
    let start = problem.start()?;
    if start.turn() != problem.first_mover() {
        return Err(Error::WrongSideToMove { expected: problem.first_mover() });
    }
    // Check that the target position parses before looking at the moves
    let target = match problem.kind {
        Kind::ProofGame => Some(Target::Placement(target_board(&problem.fen)?)),
        Kind::Release => Some(Target::release(&problem.fen)?),
        Kind::Helpmate => None,
    };
    // Release has no fixed length
    if problem.kind != Kind::Release {
        let expected = problem.plies();
        if moves.len() != expected {
            return Err(Error::WrongLength { expected, got: moves.len() });
        }
    }

    // A mate or stalemate partway through leaves no legal next move, so for helpmates
    // this guarantees "mate on exactly the last move". In both forms the last move is
    // White's, and Black is the side mated.
    let (end, ucis) = replay(problem, moves)?;
    match target {
        None => {
            if !(end.is_checkmate() && end.turn() == Color::Black) {
                return Err(Error::NotCheckmate);
            }
        }
        Some(target) => {
            if !target.matches(&end) {
                return Err(Error::NotReached);
            }
        }
    }

    for known in &problem.exclude {
        // Normalize known solutions the same way before comparing (so notation changes can't cheat).
        if let Ok((_, known_ucis)) = replay(problem, known) {
            if known_ucis == ucis {
                return Err(Error::SameAsExcluded);
            }
        }
    }
    Ok(ucis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(fen: &str, n: u32) -> Problem {
        Problem { kind: Kind::Helpmate, fen: fen.into(), n, half: false, exclude: vec![], k: 1 }
    }
    fn ph(fen: &str, n: u32) -> Problem {
        Problem { kind: Kind::Helpmate, fen: fen.into(), n, half: true, exclude: vec![], k: 1 }
    }
    fn pg(fen: &str, n: u32, half: bool) -> Problem {
        Problem { kind: Kind::ProofGame, fen: fen.into(), n, half, exclude: vec![], k: 1 }
    }
    fn mv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }
    /// Checks a single solution.
    fn v(problem: &Problem, moves: &[String]) -> Result<(), Error> {
        verify_solution(problem, moves).map(|_| ())
    }
    /// Checks several solutions with k set to their count.
    fn vk(mut problem: Problem, sols: &[&str]) -> Result<(), Error> {
        problem.k = sols.len() as u32;
        let sols: Vec<Vec<String>> = sols.iter().map(|s| mv(s)).collect();
        verify(&problem, &sols)
    }

    // Black Kh8 / White Kg6 Ra1. 1...Kg8 2.Ra8#
    const H1: &str = "7k/8/6K1/8/8/8/8/R7 b - - 0 1";

    #[test]
    fn accepts_correct_h1_in_uci_and_san() {
        assert_eq!(v(&p(H1, 1), &mv("h8g8 a1a8")), Ok(()));
        assert_eq!(v(&p(H1, 1), &mv("Kg8 Ra8#")), Ok(()));
    }

    #[test]
    fn rejects_non_mate() {
        assert_eq!(v(&p(H1, 1), &mv("h8g8 a1a7")), Err(Error::NotCheckmate));
    }

    #[test]
    fn rejects_illegal_move() {
        assert!(matches!(
            v(&p(H1, 1), &mv("h8h7 a1a8")),
            Err(Error::IllegalMove { ply: 1, .. })
        ));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(matches!(v(&p(H1, 2), &mv("h8g8 a1a8")), Err(Error::WrongLength { .. })));
    }

    #[test]
    fn rejects_white_to_move() {
        let fen = "7k/8/6K1/8/8/8/8/R7 w - - 0 1";
        assert_eq!(
            v(&p(fen, 1), &mv("a1a8 h8g8")),
            Err(Error::WrongSideToMove { expected: Color::Black })
        );
    }

    // h#1.5 with White Kf6 Ra1 / Black Kh8. 1.Kg6 Kg8 2.Ra8#
    const H15: &str = "7k/8/5K2/8/8/8/8/R7 w - - 0 1";

    #[test]
    fn accepts_h1_5() {
        assert_eq!(v(&ph(H15, 1), &mv("Kg6 Kg8 Ra8#")), Ok(()));
        assert_eq!(ph(H15, 1).label(), "h#1.5");
        assert_eq!(ph(H15, 1).plies(), 3);
    }

    #[test]
    fn h_n_5_needs_white_to_move_and_odd_length() {
        assert_eq!(
            v(&ph(H1, 1), &mv("h8g8 a1a8")),
            Err(Error::WrongSideToMove { expected: Color::White })
        );
        assert_eq!(
            v(&ph(H15, 1), &mv("Kg6 Kg8")),
            Err(Error::WrongLength { expected: 3, got: 2 })
        );
    }

    #[test]
    fn h0_5_is_a_direct_mate_in_one() {
        // White Kg6 Ra1 / Black Kh8, White to move: 1.Ra8#
        let fen = "7k/8/6K1/8/8/8/8/R7 w - - 0 1";
        assert_eq!(v(&ph(fen, 0), &mv("Ra8")), Ok(()));
    }

    // h#2 with Black Kh8 / White Kg5 Ra1. It has four solutions (a cook example).
    const H2: &str = "7k/8/8/6K1/8/8/8/R7 b - - 0 1";

    #[test]
    fn h2_has_several_solutions() {
        for sol in [
            "h8h7 g5f6 h7h6 a1h1",
            "h8g8 g5f6 g8f8 a1a8",
            "h8g8 g5g6 g8h8 a1a8",
            "h8g8 g5h6 g8h8 a1a8",
        ] {
            assert_eq!(v(&p(H2, 2), &mv(sol)), Ok(()), "{sol}");
        }
    }

    #[test]
    fn cook_differs_from_author_solution() {
        let mut prob = p(H2, 2);
        prob.exclude = vec![mv("Kg8 Kg6 Kh8 Ra8")]; // treat as the author's solution
        assert_eq!(v(&prob, &mv("Kh7 Kf6 Kh6 Rh1")), Ok(()));
        assert_eq!(v(&prob, &mv("h8g8 g5g6 g8h8 a1a8")), Err(Error::SameAsExcluded));
    }

    #[test]
    fn exclude_blocks_known_solution_even_in_other_notation() {
        let mut prob = p(H1, 1);
        prob.exclude = vec![mv("Kg8 Ra8")];
        assert_eq!(v(&prob, &mv("h8g8 a1a8")), Err(Error::SameAsExcluded));
    }

    // ---- Proof games ----

    // Position after 1.e4 e5 2.Ke2 Ke7
    const PG2: &str = "rnbq1bnr/ppppkppp/8/4p3/4P3/8/PPPPKPPP/RNBQ1BNR w - - 2 3";

    #[test]
    fn accepts_proof_game() {
        assert_eq!(v(&pg(PG2, 2, false), &mv("e4 e5 Ke2 Ke7")), Ok(()));
        assert_eq!(v(&pg(PG2, 2, false), &mv("e2e4 e7e5 e1e2 e8e7")), Ok(()));
        // The placement field alone is also accepted
        let placement = PG2.split_whitespace().next().unwrap();
        assert_eq!(v(&pg(placement, 2, false), &mv("e4 e5 Ke2 Ke7")), Ok(()));
        assert_eq!(pg(PG2, 2, false).label(), "PG 2.0");
    }

    #[test]
    fn proof_game_half_move() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR";
        assert_eq!(v(&pg(fen, 0, true), &mv("e4")), Ok(()));
        assert_eq!(pg(fen, 0, true).label(), "PG 0.5");
        // 1.e3 does not reach this placement
        assert_eq!(v(&pg(fen, 0, true), &mv("e3")), Err(Error::NotReached));
    }

    #[test]
    fn proof_game_rejects_wrong_position_and_length() {
        assert!(matches!(
            v(&pg(PG2, 2, false), &mv("e4 e5 Kd1 Ke7")),
            Err(Error::IllegalMove { ply: 3, .. })
        ));
        assert_eq!(v(&pg(PG2, 2, false), &mv("e4 e6 Ke2 Ke7")), Err(Error::NotReached));
        assert!(matches!(
            v(&pg(PG2, 2, false), &mv("e4 e5 Ke2")),
            Err(Error::WrongLength { expected: 4, got: 3 })
        ));
    }

    #[test]
    fn proof_game_back_to_initial_position_with_exclude() {
        // PG 2.0 back to the initial position: many knight out-and-back sequences work
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
        let mut prob = pg(start, 2, false);
        prob.exclude = vec![mv("Nf3 Nf6 Ng1 Ng8")];
        assert_eq!(v(&prob, &mv("g1f3 g8f6 f3g1 f6g8")), Err(Error::SameAsExcluded));
        assert_eq!(v(&prob, &mv("Nc3 Nc6 Nb1 Nb8")), Ok(()));
    }

    #[test]
    fn proof_game_rejects_bad_target() {
        assert_eq!(v(&pg("not a fen", 1, false), &mv("e4 e5")), Err(Error::BadFen));
    }

    // ---- Release the position ----

    fn rel(fen: &str) -> Problem {
        Problem { kind: Kind::Release, fen: fen.into(), n: 0, half: false, exclude: vec![], k: 1 }
    }

    // T. Orban's PG 4.0 position: the black king stands on e8 but has moved, so Black cannot castle.
    const ORBAN: &str = "rnbqkbnr/pp3ppp/2p1p3/8/4P3/8/PPPP1PPP/RNBQK1NR";

    #[test]
    fn release_accepts_games_of_any_length() {
        assert_eq!(rel(ORBAN).label(), "Release");
        assert_eq!(v(&rel(ORBAN), &mv("e4 e6 Bb5 Ke7 Bxd7 c6 Be8 Kxe8")), Ok(()));
        // A longer game reaching the same placement is fine too
        assert_eq!(
            v(&rel(ORBAN), &mv("Nf3 Nf6 Ng1 Ng8 e4 e6 Bb5 Ke7 Bxd7 c6 Be8 Kxe8")),
            Ok(())
        );
        assert_eq!(v(&rel(ORBAN), &mv("e4 e6 Bb5 Ke7")), Err(Error::NotReached));
    }

    #[test]
    fn release_full_fen_compares_turn_castling_and_ep() {
        let game = mv("e4 e6 Bb5 Ke7 Bxd7 c6 Be8 Kxe8");
        let full = |rest: &str| rel(&format!("{ORBAN} {rest}"));
        assert_eq!(v(&full("w KQ - 0 5"), &game), Ok(()));
        // Black has lost the right to castle
        assert_eq!(v(&full("w KQkq - 0 5"), &game), Err(Error::NotReached));
        // Wrong side to move
        assert_eq!(v(&full("b KQ - 0 5"), &game), Err(Error::NotReached));
        // Move counters are ignored
        assert_eq!(v(&full("w KQ - 7 42"), &game), Ok(()));

        // After 1.e4 Nf6 2.e5 d5, White can capture en passant on d6
        let ep = "rnbqkb1r/ppp1pppp/5n2/3pP3/8/8/PPPP1PPP/RNBQKBNR";
        let game = mv("e4 Nf6 e5 d5");
        assert_eq!(v(&rel(&format!("{ep} w KQkq d6 0 3")), &game), Ok(()));
        assert_eq!(v(&rel(&format!("{ep} w KQkq - 0 3")), &game), Err(Error::NotReached));
        // With a quiet last move instead, the same placement has no en passant
        let game = mv("e4 d5 e5 Nf6 Nf3 Ng8 Ng1 Nf6");
        assert_eq!(v(&rel(&format!("{ep} w KQkq - 0 5")), &game), Ok(()));
        assert_eq!(v(&rel(&format!("{ep} w KQkq d6 0 5")), &game), Err(Error::NotReached));
    }

    #[test]
    fn release_castling_rights_lost_by_rook_moves() {
        // Rooks go out and back: the placement is the initial one, but only queenside castling remains
        let game = mv("Nf3 Nf6 Rg1 Rg8 Rh1 Rh8 Ng1 Ng8");
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
        assert_eq!(v(&rel(&format!("{start} w Qq - 0 5")), &game), Ok(()));
        assert_eq!(v(&rel(&format!("{start} w KQkq - 0 5")), &game), Err(Error::NotReached));
    }

    #[test]
    fn release_rejects_illegal_moves_and_bad_fen() {
        assert!(matches!(
            v(&rel(ORBAN), &mv("e4 e6 Bb5 Ke7 Bxd7 c6 Be8 Kd8")),
            Err(Error::IllegalMove { ply: 8, .. })
        ));
        assert_eq!(v(&rel("not a fen"), &mv("e4")), Err(Error::BadFen));
        assert_eq!(v(&rel(&format!("{ORBAN} x KQ - 0 5")), &mv("e4")), Err(Error::BadFen));
    }

    // ---- k distinct solutions ----

    #[test]
    fn accepts_k_distinct_solutions() {
        let all = ["Kh7 Kf6 Kh6 Rh1", "Kg8 Kf6 Kf8 Ra8", "Kg8 Kg6 Kh8 Ra8", "Kg8 Kh6 Kh8 Ra8"];
        assert_eq!(vk(p(H2, 2), &all[..1]), Ok(()));
        assert_eq!(vk(p(H2, 2), &all[..2]), Ok(()));
        assert_eq!(vk(p(H2, 2), &all), Ok(()));
    }

    #[test]
    fn rejects_duplicate_even_in_other_notation() {
        assert_eq!(
            vk(p(H2, 2), &["Kg8 Kg6 Kh8 Ra8", "Kh7 Kf6 Kh6 Rh1", "h8g8 g5g6 g8h8 a1a8"]),
            Err(Error::Duplicate { first: 1, second: 3 })
        );
    }

    #[test]
    fn reports_which_solution_is_wrong() {
        assert_eq!(
            vk(p(H2, 2), &["Kg8 Kg6 Kh8 Ra8", "Kg8 Kg6 Kh8 Ra7"]),
            Err(Error::InSolution { index: 2, error: Box::new(Error::NotCheckmate) })
        );
    }

    #[test]
    fn rejects_wrong_count_and_zero_k() {
        let mut prob = p(H2, 2);
        prob.k = 2;
        assert_eq!(
            verify(&prob, &[mv("Kg8 Kg6 Kh8 Ra8")]),
            Err(Error::WrongCount { expected: 2, got: 1 })
        );
        prob.k = 0;
        assert_eq!(verify(&prob, &[]), Err(Error::ZeroK));
    }

    #[test]
    fn k_solutions_must_all_differ_from_excluded() {
        let mut prob = p(H2, 2);
        prob.exclude = vec![mv("Kg8 Kg6 Kh8 Ra8")];
        assert_eq!(vk(prob.clone(), &["Kh7 Kf6 Kh6 Rh1", "Kg8 Kf6 Kf8 Ra8"]), Ok(()));
        assert_eq!(
            vk(prob, &["Kh7 Kf6 Kh6 Rh1", "Kg8 Kg6 Kh8 Ra8"]),
            Err(Error::InSolution { index: 2, error: Box::new(Error::SameAsExcluded) })
        );
    }

    #[test]
    fn k_distinct_proof_game_sequences() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
        assert_eq!(
            vk(pg(start, 2, false), &["Nf3 Nf6 Ng1 Ng8", "Nc3 Nc6 Nb1 Nb8", "Nf3 Nc6 Ng1 Nb8"]),
            Ok(())
        );
    }

    // ---- Placement-only FEN and ambiguous SAN ----

    #[test]
    fn placement_only_fen_gets_side_to_move_from_form() {
        // h#1: Black to move is filled in
        assert_eq!(v(&p("7k/8/6K1/8/8/8/8/R7", 1), &mv("Kg8 Ra8")), Ok(()));
        // h#1.5: White to move is filled in
        assert_eq!(v(&ph("7k/8/5K2/8/8/8/8/R7", 1), &mv("Kg6 Kg8 Ra8")), Ok(()));
        // A full FEN with the wrong side to move is still rejected
        assert_eq!(
            v(&p("7k/8/6K1/8/8/8/8/R7 w - - 0 1", 1), &mv("a1a8 h8g8")),
            Err(Error::WrongSideToMove { expected: Color::Black })
        );
    }

    #[test]
    fn ambiguous_san_lists_candidates() {
        // Black knights on c6 and c2 can both go to d4
        let prob = p("2N2NK1/8/2n1P3/2p1P3/4k1bR/8/2n5/1B6", 2);
        match v(&prob, &mv("Kd5 Rh5 Nd4 Nb6")) {
            Err(Error::AmbiguousMove { ply: 3, mv, mut candidates }) => {
                assert_eq!(mv, "Nd4");
                candidates.sort();
                assert_eq!(candidates, ["N2d4", "N6d4"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(v(&prob, &mv("Kd5 Rh5 N2d4 Nb6")), Ok(()));
        assert_eq!(v(&prob, &mv("Kd5 Rh5 c2d4 Nb6")), Ok(()));
    }
}

