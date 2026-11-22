// Program that runs inside the zkVM.
// Verifies k secret, pairwise distinct solutions and writes only the problem (including k)
// and the claimant to the journal.
use helpplay_core::{verify, Claim, Input};
use risc0_zkvm::guest::env;

fn main() {
    let input: Input = env::read();

    if let Err(e) = verify(&input.problem, &input.solutions) {
        // A panic means no proof is produced
        panic!("invalid solutions: {e}");
    }

    // The solutions (input.solutions) are not written.
    env::commit(&Claim { problem: input.problem, claimant: input.claimant });
}
