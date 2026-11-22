# helpplay-zkp

A RISC Zero program that proves you **know k distinct solutions** (k = 1 by default) to a helpmate (h#n / h#n.5) or a proof game (PG n.0 / n.5) **without revealing them**.
Proofs are produced in Groth16 format.

## Layout

| Path | Role |
|---|---|
| `core/` | Verification logic (legal moves, move count, checkmate or match with the target position, solutions pairwise distinct and different from known solutions). Shared by guest and host |
| `methods/guest/` | Program that runs inside the zkVM. Verifies the solutions and writes **only the problem (including k) and the claimant** to the journal |
| `host/` | CLI that creates (`prove`) and verifies (`verify`) proofs. `host/tests/regression.rs` holds the regression tests |
| `examples/` | Example problems and solutions (see the table below) |

| Example | Contents |
|---|---|
| `hmazemmour25.json` / `hmazemmour25_sol.txt` | H. Maeshima, The 11th Azemmour Tournament 2025: h#2, two solutions (k = 2). Also a regression test |
| `hmhopper25.json` / `hmhopper25_sol.txt` | H. Maeshima, The Hopper 2025: h#4.5. Also a regression test |
| `pg_problem.json` / `pg_solution.txt` | T. Orban, Die Schwalbe 1976: proof game PG 4.0 |

Public: the problem type, the position (FEN), the number of moves, k, excluded known solutions, the claimant
Secret: the solutions (the moves)

## Setup

```sh
curl -L https://risczero.com/install | bash
rzup install
```

Converting proofs to Groth16 format requires **Docker** (x86_64). Without an NVIDIA GPU, RISC Zero runs the conversion in a Docker container.

## Problem file (problem.json)

`examples/hmazemmour25.json`:

```json
{
    "author": "Maeshima, Hiroaki",
    "source": "The 11th Azemmour Tournament",
    "year": 2025,
    "award": "1st-2nd Honorable Mention",
    "fen":"2N2NK1/8/2n1P3/2p1P3/4k1bR/8/2n5/1B6",
    "n":2,
    "k":2
}
```

| Key | Meaning | Default |
|---|---|---|
| `kind` | `"helpmate"` or `"proof_game"` | `"helpmate"` |
| `fen` | Helpmate: the starting position. Proof game: the target position. In both, the piece placement alone is enough (see below) | (required) |
| `n` | Move count, written as `x`, `x.0` or `x.5`: `x` and `x.0` mean 2x plies, `x.5` means 2x+1 plies. A number (`2.5`) or a string (`"2.5"`) | (required) |
| `k` | Number of pairwise distinct solutions to prove knowledge of | `1` |
| `exclude` | Known solutions to exclude (e.g. the author's solution) | none |
| `author` | Metadata: the composer, or a list for joint compositions | none |
| `source` | Metadata: where the problem was published (e.g. a magazine) | none |
| `year` | Metadata: year of publication, a number or a string (`2025`, `"2024-25"`) | none |
| `award` | Metadata: honours such as `"1st Prize"` | none |
| `note` | Metadata: free-form note | none |

The metadata keys are **ignored by the proof** and not written to the journal, so adding or editing them does not affect existing receipts.
`verify` prints them in one line (`problem: Maeshima, Hiroaki, The 11th Azemmour Tournament 2025, 1st-2nd Honorable Mention`; `note` is not shown).
For joint compositions, give `author` as a list; the names are joined with ` & `.
Any other key is an error, to catch typos.

For a helpmate, if `fen` has only the piece placement (e.g. `"2N2NK1/8/2n1P3/2p1P3/4k1bR/8/2n5/1B6"`),
the side to move follows from `n` (Black for h#x, White for h#x.5), with no castling rights and no en passant.
If you write a full FEN, its side to move must match the form.

The solution file (solution.txt) has **one solution per line**, with moves separated by whitespace, in either UCI (`e2e4`) or SAN (`e4`, `Ra8#`).
Blank lines are ignored. The file must contain exactly `k` solutions. `examples/hmazemmour25_sol.txt`:

```
Kd5 Rh5 N2d4 Nb6#
Kxe5 Ba2 Bf5 Nd7#
```

SAN must be disambiguated when two pieces of the same type can reach the square (`N2d4`, not `Nd4`); otherwise the error lists the candidates:

```
Error: solutions are not valid: solution 1: ply 3 (Nd4) is ambiguous; write one of: N2d4, N6d4
```

## Usage

```sh
# Create a proof (the solution file stays secret)
cargo run --release -p host -- prove examples/hmhopper25.json examples/hmhopper25_sol.txt alice receipt.bin

# Verify it (only the problem file and receipt.bin are needed)
cargo run --release -p host -- verify examples/hmhopper25.json receipt.bin
# => OK: alice knows a solution of h#4.5
#        problem: Maeshima, Hiroaki, The Hopper 2025
#        receipt format: Groth16
```

### Log output

All messages, including errors, go through a log handler. By default they are written to **stdout**.
Choose another destination with `--log` (anywhere on the command line):

| `--log` | Destination |
|---|---|
| `stdout` (default) | Standard output |
| `stderr` | Standard error |
| `none` | Nothing is written |
| a file path | Appended to that file |

```sh
cargo run --release -p host -- --log run.log verify examples/hmhopper25.json receipt.bin
```

`--log-level` sets the lowest level written: `debug`, `info` (default), `warn` or `error`.
With `debug`, the time of each step is logged, and `prove` also logs the zkVM cycle counts:

```
$ cargo run --release -p host -- --log-level debug prove examples/hmazemmour25.json examples/hmazemmour25_sol.txt alice r.bin
Debug: time: load problem: 40.941µs
Debug: time: load solutions: 11.662µs
Debug: time: check solutions: 47.599µs
Debug: time: prove (zkVM + Groth16): 136.312ms
Debug: stats: 1 segment(s), 131072 total cycles (53368 user, 61738 paging, 15966 reserved)
Debug: time: write receipt: 144.915µs
receipt written to r.bin
Debug: time: total: 136.738ms
```

(These times are from dev mode; a real Groth16 proof takes minutes.)

The exit code is 0 on success and 1 on failure, whatever the destination.
Messages that RISC Zero itself prints (such as the dev mode warning) do not go through the handler.
Programs using the `host` library can install their own handler with `host::logging::set_handler`
(implement the `LogHandler` trait; `MemoryHandler` keeps messages in memory).

`prove` checks the solution locally before running the zkVM and fails immediately if it is wrong.
Proving takes several minutes to over ten minutes (the first run also downloads the Docker image).

### h#n.5

Write `n` as `x.5` (e.g. `"n": 1.5`) for h#x.5. White is to move in the starting position, and Black is mated after 2x+1 plies starting with White.
With `x` or `x.0`, Black is to move in the starting position and Black is mated after 2x plies.

`examples/hmhopper25.json` is an h#4.5 (`"n":4.5`); its solution has 9 plies starting with White:

```
Bg1 Be1 Be3 Bf2 Bg5 Kg1 Bf4 Nh1 Bh2#
```

### Proof games

Set `"kind": "proof_game"` for a proof game. Play starts from the standard initial position, and the proof shows
that you know a sequence of moves reaching the position given in `fen`.

- Move count: `"n": 4` or `"n": 4.0` is PG 4.0 (8 plies), and `"n": 3.5` is PG 3.5 (7 plies). White moves first in both.
- **Only the piece placement** (the first field of the FEN) is compared, so you may give just the placement. The side to move follows from the move count; castling rights and en passant are not compared.

```sh
cargo run --release -p host -- prove examples/pg_problem.json examples/pg_solution.txt alice pg.bin
cargo run --release -p host -- verify examples/pg_problem.json pg.bin
# => OK: alice knows a solution of PG 4.0
#        problem: Orban, Tibor, Die Schwalbe 1976
```

### k distinct solutions

Set `"k"` to prove that you know k pairwise distinct solutions. Each solution is checked on its own, and every pair must differ.
Moves are normalized to UCI before comparison, so writing the same sequence in another notation does not count as a different solution.

```sh
cargo run --release -p host -- prove examples/hmazemmour25.json examples/hmazemmour25_sol.txt alice azemmour.bin
cargo run --release -p host -- verify examples/hmazemmour25.json azemmour.bin
# => OK: alice knows 2 distinct solutions of h#2
#        problem: Maeshima, Hiroaki, The 11th Azemmour Tournament 2025, 1st-2nd Honorable Mention
```

`k` is part of the problem, so a receipt only verifies against a problem.json with the same `k`.
Combined with `exclude`, every one of the k solutions must also differ from the excluded ones.

### Pointing out cooks

Putting the author's solution in `exclude` turns the proof into "I know a solution different from that one". This works for both helpmates and proof games.
Moves are normalized internally regardless of UCI / SAN before comparison, so changing the notation cannot slip past the check.

### Development

With `RISC0_DEV_MODE=1`, proving is skipped so you can try things quickly (Docker is not needed either).
The resulting receipt is fake, however, and `verify` rejects it when `RISC0_DEV_MODE` is not set.

```sh
RISC0_DEV_MODE=1 cargo run --release -p host -- prove examples/hmhopper25.json examples/hmhopper25_sol.txt alice receipt.bin
RISC0_DEV_MODE=1 cargo run --release -p host -- verify examples/hmhopper25.json receipt.bin
cargo test -p helpplay-core   # tests for the verification logic (runs without the zkVM)
```

### Regression tests

```sh
cargo regression
```

This is an alias (defined in `.cargo/config.toml`) for `cargo test --release -p host --test regression`.
For each case in `host/tests/regression.rs`, it loads the problem and solution files from `examples/` the same way as the CLI,
checks them, and runs the guest in the zkVM executor (no proof is made, so it takes seconds, not minutes).
To add a case, put the files in `examples/` and add a test function that calls `run(Case { ... })`.

## Notes

- **claimant**: Binds the proof to a name or address, so nobody else can take your receipt and claim it as their own proof.
- **ImageID**: `verify` checks against `HELPPLAY_GUEST_ID`. If someone else will verify your proofs, share the ImageID with them in advance.
  Changing `core/` or `methods/guest/` changes the ImageID, and receipts created before the change can no longer be verified.
- **Receipt format**: receipt.bin is in Rust's bincode format. Verifying it in a smart contract requires extracting the seal and the journal separately (not yet supported).
- **Zero knowledge**: RISC Zero receipts are said to be zero-knowledge, but check the documentation for the version you use.
- **Little value for small problems**: if the move count is small, a verifier can search all moves and solve the problem themselves.
- Supported forms are h#n (Black first, 2n plies), h#n.5 (White first, 2n+1 plies), and proof games PG n.0 / n.5. Series movers are not supported.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you,
as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
