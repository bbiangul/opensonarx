# OpenSonarX Economic Model

> "Stake truth, burn spam, earn trust"

## Token Supply & Emission

```
Admin mints $TRUTH ──► max_total_supply cap (1B default, enforced on every mint)

Epoch 0:   10,000,000 max emission
Epoch 1:    9,850,000    (1.5% decay per epoch)
Epoch 52:  ~4,557,000    (Year 1 — still healthy rewards)
Epoch 104: ~2,077,000    (Year 2)
Epoch 156:   ~946,000    (Year 3)
  ...          ...       (converges to zero)

Min epoch duration: 7 days (prevents rapid exhaustion)
Emission schedule spans ~3 years (with 7-day epochs)
```

## Emission Pipeline

```
FundRewards (anyone)
    │
    │  Transfers $TRUTH into reward vault (one-way, no withdrawal)
    │  Acts as rate-limiter, NOT a reward pool
    ▼
UpdateRewardIndex (permissionless)
    │
    │  Caps at: min(requested, distributable, epoch_budget,
    │               supply_headroom)
    │
    ├────── 70% ──────►  Staker Reward Index
    │                    (reward_per_token_stored, 1e18 precision)
    │                    Divided proportionally by total_staked
    │
    └────── 30% ──────►  Sentinel Reward Index
           + fees        (sentinel_reward_per_token)
                         Divided proportionally by sentinel_staked

Safety: If no stakers/sentinels exist, emissions are NOT counted
        against the cap — they remain available for later
```

## Who Stakes & Why

### Publishers (Staker role)

- Stakes $TRUTH on a domain (e.g. `example.com`)
- Vouches for content quality
- Earns 70% of emissions (pro-rata)
- Risk: slashed if domain disputed & loses
- Min stake enforced (`config.min_stake`)

### Curators (also Staker role)

- Stakes on domains they don't own
- Yield capped at 15% APY (1500 bps) — prevents whale-gaming emission pool
- UGC platforms: curators get 0 passthrough

### Sentinels (Quality police)

- Stakes into sentinel pool (no specific domain)
- Earns 30% of emissions + 5% of channel settlement fees
- Can file disputes against bad domains
- Min stake: `config.jury_min_stake`
- **Trusted reporters** (stake >= `jury_min_stake * 10`) get 50% dispute bond discount

## Dispute / Slashing

```
Sentinel files dispute
    │
    │  Posts dispute_bond as collateral
    │  (50% discount for trusted reporters with high stake)
    ▼
Jury Voting (commit-reveal, 2 phases)
    │
    │  Jurors = stakers with enough stake + 7-day age
    │  Must commit hash first, then reveal vote
    │  Minimum jury quorum required (min_jury_weight, default 1000)
    │  Winning jurors earn flat jury_vote_reward (minted from supply)
    ▼
Resolution
    │
    ├── Domain GUILTY ──►  Graduated slashing:
    │                      Low severity:  50% of slash_pct
    │                      Med severity: 100% of slash_pct
    │                      High severity: 150% of slash_pct
    │
    │                      Slashed tokens → 50% burned (deflationary)
    │                                       50% to protocol revenue (vault)
    │                      Reporter gets full bond back
    │
    └── Domain INNOCENT ► Reporter loses bond (discourages spam)
                           Bond stays in vault as protocol revenue
                           Domain stakers unaffected

Jury rewards: Flat fee (jury_vote_reward) minted per winning juror
              from supply cap — no profit motive from verdict outcome
              Capped per epoch (max_jury_rewards_per_epoch) to prevent farming

KEY: Stakers CANNOT unstake during active dispute (S1 fix)
     This prevents front-running slashing by withdrawing early
```

## Payment Channels

```
AI Agent (payer) ◄──────── state channel ────────► Publisher (payee)

On settlement:
    │
    ├── 10% BURNED  (deflationary pressure, removes spam incentive)
    ├──  5% Sentinel fee (accumulated, drains into sentinel index)
    └── Remainder → Payee
```

Why burn? AI agents pay for search results. Burning a portion of the payment makes spam unprofitable — you spend more than you earn. The burn rate starts low (10%) to bootstrap publisher adoption and can be raised via governance as the network matures.

## Governance

- Stakers create & vote on proposals (parameter changes)
- Vote weight = staked amount
- Voting period: **24 hours**
- **Execution timelock**: 24-hour delay after voting ends before changes take effect (rage-quit window, minimum 1 hour — cannot be zeroed)
- **7-day stake age** required to vote (prevents flash-loan governance attacks)
- Voting locks **all** your positions (per-wallet lock)
- Cannot unstake ANY position until vote period ends
- Execution after `execution_delay_secs`
- Bounded ranges on all parameters (slash_pct: 5-100%, execution_delay: 1h-30d, etc.)
- **Proposal deposit** (100 $TRUTH default, refunded on execution, lost if canceled) — prevents spam proposals
- Governs: `min_stake`, `slash_pct`, `burn_pct`, `staker_emission_pct`, `curator_yield_cap_bps`, `jury_vote_reward`, `max_jury_rewards_per_epoch`, `sentinel_fee_pct`, `execution_delay_secs`, `proposal_deposit`, `min_jury_weight`, `max_total_supply`, etc.

## Default Parameters

| Parameter | Default | Range | Notes |
|-----------|---------|-------|-------|
| `burn_pct` | 10% | 0-100% | Channel settlement burn |
| `sentinel_fee_pct` | 5% | 0-50% | Channel fee to sentinels (burn + sentinel must be <= 100%) |
| `slash_pct` | configurable | 5-100% | Base slash rate (graduated by severity) |
| `staker_emission_pct` | 70% | 0-100% | Staker share of emissions (sentinel gets remainder) |
| `curator_yield_cap_bps` | 1500 | 0-10000 | 15% APY cap for curators |
| `execution_delay_secs` | 86,400 | 3,600-2,592,000 | 24h default timelock (min 1h, max 30d) |
| `proposal_deposit` | 100 | 0-1B | Anti-spam deposit for governance proposals |
| `min_jury_weight` | 1,000 | 0-10B | Minimum jury weight for dispute quorum |
| `max_total_supply` | 1,000,000,000 | >= total_minted | Hard cap on total mintable $TRUTH |
| `jury_vote_reward` | 1,000 | 0-1B | Flat reward per winning juror |
| `max_jury_rewards_per_epoch` | 50,000 | 0-1B | Per-epoch jury reward budget |
| `min_epoch_duration` | 604,800 | 0-365d | 7 days between epoch advances |

## Why It Works

### 1. Skin in the Game

Publishers must stake real value. Spam = risk of slashing. No free lunch for polluters.

### 2. Deflationary Pressure

10% of channel payments burned + 50% of slashed tokens burned = shrinking supply. Emission decays 1.5% per epoch → gradual transition to fee-driven economy over ~3 years.

### 3. Aligned Incentives

| Actor | Incentive |
|-------|-----------|
| Publisher | Stake on quality content → earn emissions |
| Curator | Find good domains → earn capped yield |
| Sentinel | Catch bad actors → earn 30% emissions + 5% channel fees + flat jury fees |
| AI Agent | Pay for accurate results → better answers |
| Voter | Improve protocol → protect stake value |

### 4. Anti-Gaming Mechanisms

- **Curator yield cap** (15%) prevents whale emission farming
- **UGC flag** blocks domain-stake passthrough on user platforms
- **Per-wallet gov lock** prevents vote-then-dump attacks
- **Dispute lock** prevents unstake-before-slash attacks
- **Epoch rate-limiting** prevents emission schedule exhaustion
- **Supply headroom cap** prevents phantom (unmintable) rewards
- **Hard supply cap** (1B default) prevents infinite inflation
- **Cooldown period** on unstaking (but rewards still claimable)
- **Dispute bond** deters frivolous challenges
- **Flat jury fee** removes profit motive from predatory slashing
- **Per-epoch jury reward cap** prevents jury farming (fake disputes to drain inflation)
- **Jury quorum** (min_jury_weight) prevents single-juror verdicts
- **7-day stake age for voting** prevents flash-loan governance attacks
- **24-hour execution timelock** (cannot be set below 1 hour) gives community time to react to malicious proposals
- **Governance parameter floors** (slash_pct >= 5%, execution_delay >= 1h) prevent zeroing critical safety mechanisms
- **Proposal deposit** (100 $TRUTH) prevents governance spam
- **Trusted reporter discount** ensures honest sentinels can handle spam waves
- **Slash burn** (50% of slashed tokens burned) ensures slashing is deflationary, not just redistributive

### 5. Sustainable Equilibrium

```
Early: High emissions attract stakers → builds index quality
       Low burn rate (10%) attracts publishers
       5% sentinel fee bootstraps quality enforcement
Mid:   Gradual decay creates scarcity → token appreciates
       Governance can raise burn/sentinel fees as network grows
Late:  Channel fees sustain sentinels → quality maintained
End:   Zero emission, fully fee-driven economy
```

**The core thesis**: Quality content is profitable (stake + earn), spam is unprofitable (stake + get slashed + payment gets burned). Sentinels are paid to enforce this via emissions, channel fees, and flat jury fees — not from slashed bonds, removing any incentive for predatory disputes. With 1.5% weekly decay, emissions stay meaningful through Year 3 (~946K/week), giving the protocol time to build organic fee revenue before inflation winds down. The burn mechanic (channel burns + slash burns) ensures the token remains scarce.
