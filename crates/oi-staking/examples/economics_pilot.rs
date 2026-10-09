//! Executable local economic workflow. No mock engine, network or money moves.
//! Scenario facts and funding are hypothetical; this is not a production pilot.
use oi_staking::pilot_economics::*;
use oi_types::Result;

fn main() -> Result<()> {
    let started = std::time::Instant::now();
    let price = minimum_service_price(10_700, 10_000_000_000, 1_000_000, 5_000, 0, 300, 3_000)?;
    println!("Scenario: 50% billable, 3% processor, 30% target margin.");
    println!("Minimum billed-event price: {price} millionths of USD.");
    assert!(price > 30_000); // Do not silently offer the old loss-making quote.
    assert!(validate_participant_payment(2_000, 3_000, 0, 0, 1).is_err());
    println!("Underfunded node quote: rejected.");
    validate_coalition_budget(100_000_000, 30_000_000, 0, 0, 0)?;
    assert!(validate_coalition_budget(100_000_000, 30_000_000, 0, 85_000_000, 0).is_err());
    println!("Reward farming bonus: rejected.");
    let mut book = CollateralBook::new(200_000_000, 100_000_000)?;
    book.admit("claim-a", "event-a", 100_000_000)?;
    book.admit("claim-b", "event-b", 100_000_000)?;
    assert!(book.admit("claim-c", "event-c", 100_000_000).is_err());
    println!(
        "Third unfunded commitment: rejected; balances {:?}.",
        book.balances()
    );
    book.close("claim-a", Outcome::Unresolved)?;
    let payout = dispute_payout(
        100_000_000,
        10_000_000,
        2_000_000,
        30_000_000,
        Outcome::Upheld,
    )?;
    book.close("claim-b", Outcome::Upheld)?;
    assert!(book.close("claim-b", Outcome::Upheld).is_err());
    println!("Unresolved author bond released; upheld bond forfeited once.");
    println!(
        "Escrow split: {payout:?}; author pool {:?}.",
        book.balances()
    );
    println!("Consumer compensation: zero (no insurance product).");
    println!(
        "Local arithmetic/transition runtime: {:?}; NOT search/chain latency.",
        started.elapsed()
    );
    Ok(())
}
