"""Pure scenario calculations. No wallets, payments, or production estimates.

Run: python3 scripts/economics_pilot.py --test
Run: python3 scripts/economics_pilot.py
All monetary inputs are illustrative USD assumptions, not observed costs.
"""
import argparse
import json
import math
import unittest


def nonnegative(**values):
    if any(not math.isfinite(value) or value < 0 for value in values.values()):
        raise ValueError("all monetary/count inputs must be finite and nonnegative")


def probability(**values):
    nonnegative(**values)
    if any(value > 1 for value in values.values()):
        raise ValueError("rates must be in [0,1]")


def dispute(bounty=30.0, bond=100.0, challenge=10.0, fee=2.0):
    nonnegative(bounty=bounty, bond=bond, challenge=challenge, fee=fee)
    if min(bounty, bond, challenge, fee) < 0 or bounty > bond or fee > challenge:
        raise ValueError("nonnegative amounts, bounty <= bond and fee <= challenge required")
    total = bond + challenge + fee
    return {
        "input": total,
        "proceeding": {"author": 0.0, "challenger": challenge + bounty,
                       "verifier": fee, "restricted_reserve": bond - bounty},
        "not_proceeding": {"author": bond + fee, "challenger": 0.0,
                           "verifier": fee, "restricted_reserve": challenge - fee},
        "unresolved": {"author": bond, "challenger": challenge,
                       "verifier": fee, "restricted_reserve": 0.0},
    }


def challenge_ev(correct_probability, investigation=10.0, bounty=30.0,
                 challenge=10.0, fee=2.0, unresolved_probability=0.0):
    probability(correct_probability=correct_probability, unresolved_probability=unresolved_probability)
    nonnegative(investigation=investigation, bounty=bounty, challenge=challenge, fee=fee)
    if not 0 <= correct_probability <= 1 or not 0 <= unresolved_probability <= 1:
        raise ValueError("probabilities must be in [0,1]")
    # correct_probability is conditional on resolution; costs occur on every attempt.
    resolved = 1 - unresolved_probability
    return resolved * (correct_probability * bounty
                       - (1 - correct_probability) * challenge) - fee - investigation


def query_economics(price, incremental_cost, queries=1_000_000,
                    historical_variable=0.0077, monthly_fixed=10_000.0):
    nonnegative(price=price, incremental_cost=incremental_cost, queries=queries,
                historical_variable=historical_variable, monthly_fixed=monthly_fixed)
    if queries <= 0 or min(price, incremental_cost, historical_variable, monthly_fixed) < 0:
        raise ValueError("positive query volume and nonnegative costs required")
    unit_cost = historical_variable + incremental_cost + monthly_fixed / queries
    revenue = price * queries
    return {"price": price, "incremental_cost_assumption": incremental_cost,
            "unit_cost": unit_cost, "monthly_revenue": revenue,
            "monthly_result": revenue - unit_cost * queries,
            "margin": (price - unit_cost) / price if price else None,
            "price_for_30pct_margin": unit_cost / 0.7}


def guarantee_premium(error=0.001, detection=1.0, lock_days=2.0,
                      commitments=5000, bond=100.0, annual_capital_cost=0.10,
                      check_cost=0.005, capital_buffer=0.20):
    probability(error=error, detection=detection)
    nonnegative(lock_days=lock_days, commitments=commitments, bond=bond,
                annual_capital_cost=annual_capital_cost, check_cost=check_cost,
                capital_buffer=capital_buffer)
    capital = commitments / 30 * lock_days * bond * (1 + capital_buffer)
    capital_cost = capital * annual_capital_cost / 12
    expected_loss = commitments * error * detection * bond
    checks = commitments * check_cost
    subtotal = capital_cost + expected_loss + checks
    return {"capital": capital, "capital_cost": capital_cost,
            "expected_loss": expected_loss, "checks": checks,
            "premium_subtotal": subtotal, "premium_with_20pct_cost_markup": subtotal * 1.2,
            "excluded": ["claim correlation", "administration", "custody", "legal/tax",
                         "capital tail loss", "independent audit procurement"]}


def service_cashflow(price=0.03, attempts=1_000_000, billable_fraction=1.0,
                     refund_rate=0.0, processor_fraction=0.0, burn_fraction=0.0,
                     costs=None, monthly_fixed=10_000.0, target_margin=0.30,
                     monthly_subsidy=0.0):
    """Disjoint illustrative costs; subsidies never establish service profitability.

    Every attempt incurs cost. Only an agreed billable event earns revenue.
    Processor fees apply to gross billing; burn to cash after refunds.
    Costs are amounts paid to named roles per attempt, not percentages of a
    token channel. Do not charge publishers/nodes twice in both cost and split.
    """
    if costs is None:
        costs = {"historical_aggregate_unallocated": .0077,
                 "additional_evidence_assumption": .003}
    nonnegative(price=price, attempts=attempts, monthly_fixed=monthly_fixed,
                monthly_subsidy=monthly_subsidy)
    nonnegative(**costs)
    probability(billable_fraction=billable_fraction, refund_rate=refund_rate,
                processor_fraction=processor_fraction, burn_fraction=burn_fraction,
                target_margin=target_margin)
    if attempts <= 0:
        raise ValueError("attempts must be positive")
    variable = sum(costs.values())
    gross = attempts * billable_fraction * price
    refunds = gross * refund_rate
    revenue = gross - refunds
    processing = gross * processor_fraction
    burned = revenue * burn_fraction
    cost_by_role = {role: value * attempts for role, value in costs.items()}
    operations = variable * attempts + monthly_fixed
    result = revenue - processing - burned - operations
    collected_per_attempt = price * billable_fraction * (
        (1 - refund_rate) * (1 - burn_fraction) - processor_fraction)
    contribution = collected_per_attempt - variable
    denominator = billable_fraction * (
        (1 - refund_rate) * (1 - burn_fraction - target_margin) - processor_fraction)
    return {
        "assumptions_not_observed": True, "attempts": attempts,
        "billable_fraction": billable_fraction, "gross_billing": gross,
        "refunds": refunds, "net_customer_revenue": revenue,
        "processor_paid": processing, "burned": burned,
        "cost_by_role": cost_by_role, "fixed_cost": monthly_fixed,
        "operating_result_excluding_subsidy": result,
        "operating_margin_on_net_customer_revenue": result / revenue if revenue else None,
        "cash_result_with_subsidy": result + monthly_subsidy,
        "price_for_target_margin": (variable + monthly_fixed / attempts) / denominator if denominator > 0 else None,
        "break_even_attempts": monthly_fixed / contribution if contribution > 0 else None,
        "consumer_indemnity_included": False,
    }


def challenger_threshold(investigation=10.0, bounty=30.0, challenge=10.0,
                         fee=2.0, unresolved_probability=0.0):
    probability(unresolved_probability=unresolved_probability)
    nonnegative(investigation=investigation, bounty=bounty, challenge=challenge, fee=fee)
    resolved = 1 - unresolved_probability
    if resolved == 0 or bounty + challenge == 0:
        return None
    threshold = (challenge + (fee + investigation) / resolved) / (bounty + challenge)
    return threshold if threshold <= 1 else None


def coalition_result(bonus=0.0, reserve_recovery=0.0, external_cost=10.0):
    """Worst case: coalition owns author, challenger and verifier.
    Emissions/rebates/other subsidies are recoverable bonus, never revenue.
    """
    probability(reserve_recovery=reserve_recovery)
    nonnegative(bonus=bonus, external_cost=external_cost)
    model = dispute()
    payout = model["proceeding"]
    recovered = payout["challenger"] + payout["verifier"] + payout["author"]
    recovered += reserve_recovery * payout["restricted_reserve"]
    return recovered + bonus - model["input"] - external_cost


def guarantee_tail(monthly_commitments=5000, cluster_size=100,
                   concurrent_bad_clusters=1, bond=100.0, reserve=40_000.0,
                   ordinary_capital=40_000.0):
    """Deterministic correlation stress, not an estimated loss probability.
    Author collateral and loss reserve are different funding needs. A full
    bond can be seized even though the consumer receives no compensation.
    """
    nonnegative(monthly_commitments=monthly_commitments, cluster_size=cluster_size,
                concurrent_bad_clusters=concurrent_bad_clusters, bond=bond,
                reserve=reserve, ordinary_capital=ordinary_capital)
    loss = min(monthly_commitments, cluster_size * concurrent_bad_clusters) * bond
    requested = min(monthly_commitments, cluster_size * concurrent_bad_clusters)
    capacity = math.floor(ordinary_capital / bond) if bond else 0
    return {"correlated_loss_scenario": loss,
            "requested_simultaneous_claims": requested,
            "fully_collateralized_capacity": capacity,
            "admission_must_reject": requested > capacity or bond == 0,
            "scenario_is_admissible": requested <= capacity and bond > 0,
            "reserve_remaining": reserve - loss,
            "reserve_shortfall": max(0, loss - reserve),
            "capital_plus_separate_reserve": ordinary_capital + reserve,
            "consumer_compensation": 0.0,
            "estimated_probability": None}


def participant_economics(payment, operating_cost, capital_cost=0.0,
                          expected_loss=0.0, token_bonus=0.0):
    """Per service unit. Token bonuses are hypothetical realizable subsidy,
    not token valuation forecasts or evidence of a sustainable participant.
    """
    nonnegative(payment=payment, operating_cost=operating_cost,
                capital_cost=capital_cost, expected_loss=expected_loss,
                token_bonus=token_bonus)
    net = payment - operating_cost - capital_cost - expected_loss
    return {"net_without_subsidy": net, "net_with_assumed_bonus": net + token_bonus,
            "sustainable_without_subsidy": net > 0,
            "minimum_payment_to_break_even": operating_cost + capital_cost + expected_loss}


def solana_fee_assumption(transactions=3, signatures_per_transaction=1,
                          priority_lamports_per_transaction=0,
                          sol_usd_assumption=100.0, queries_per_batch=1):
    nonnegative(transactions=transactions, signatures_per_transaction=signatures_per_transaction,
                priority_lamports_per_transaction=priority_lamports_per_transaction,
                sol_usd_assumption=sol_usd_assumption, queries_per_batch=queries_per_batch)
    if queries_per_batch <= 0:
        raise ValueError("batch size must be positive")
    # Official base fee 5000 lamports/signature. USD price is hypothetical.
    lamports = transactions * (5000 * signatures_per_transaction + priority_lamports_per_transaction)
    return {"network_fee_usd_per_query": lamports / 1e9 * sol_usd_assumption / queries_per_batch,
            "sol_usd_is_assumption": True,
            "excluded": ["RPC", "account funding/rent", "custody", "extra signatures", "additional paid retry transactions"]}


def scenarios():
    return {
        "status": "proposed_assumptions_not_production_validation",
        "legacy_dispute": dispute(bounty=10), "proposed_dispute": dispute(),
        "legacy_correct_ev_cost10": challenge_ev(1, bounty=10),
        "proposed_correct_ev_cost10": challenge_ev(1),
        "proposed_ev_at_90pct_cost10": challenge_ev(0.9),
        "proposed_break_even_correct_probability_cost10": 22 / 40,
        "query_scenarios": [query_economics(price, extra)
                            for extra in [0.001, 0.003, 0.010]
                            for price in [0.02, 0.03]],
        "guarantee_100pct_detection": guarantee_premium(),
        "guarantee_1pct_error": guarantee_premium(error=0.01),
        "guarantee_14day_lock": guarantee_premium(lock_days=14),
        "billable_sensitivity": [service_cashflow(billable_fraction=b)
                                 for b in [0, .25, .5, .75, 1]],
        "pilot_volume_sensitivity": [service_cashflow(attempts=n)
                                     for n in [10_000, 100_000, 1_000_000]],
        "processor_and_burn_stress": service_cashflow(processor_fraction=.03, burn_fraction=.10),
        "illustrative_role_costs": service_cashflow(costs={
            "operator_compute_discovery": .0107, "serving_nodes": .002,
            "licensed_publisher_delivery": .004, "verification_nodes": .001,
            "chain_and_billing": .001}, processor_fraction=.03),
        "ambiguity_sensitivity": [{"unresolved": u, "ev_at_q90": challenge_ev(.9, unresolved_probability=u),
                                   "minimum_precision": challenger_threshold(unresolved_probability=u)}
                                  for u in [0, .25, .5, .75, 1]],
        "coalition_no_bonus": coalition_result(),
        "coalition_subsidy_attack": coalition_result(bonus=85),
        "coalition_reserve_capture": coalition_result(bonus=25, reserve_recovery=.8),
        "correlation_stress": [guarantee_tail(cluster_size=size) for size in [100, 500, 1000]],
        "admissible_tail_exhausts_reserve": guarantee_tail(cluster_size=400, reserve=30_000),
        "serving_node_cost_sensitivity": [participant_economics(.002, cost)
                                          for cost in [.001, .002, .003]],
        "verifier_underfunded": participant_economics(2, 3),
        "node_bonus_hides_loss": participant_economics(.002, .003, token_bonus=.002),
        "additional_reserve_carry_monthly_assumption": 40_000 * .10 / 12,
        "onchain_per_query": solana_fee_assumption(),
        "onchain_batched_100": solana_fee_assumption(queries_per_batch=100),
    }


class ModelTests(unittest.TestCase):
    def test_invalid_inputs_do_not_manufacture_profit(self):
        for invalid in [float("nan"), float("inf"), -1]:
            with self.assertRaises(ValueError): dispute(bond=invalid)
            with self.assertRaises(ValueError): guarantee_premium(bond=invalid)
            with self.assertRaises(ValueError): service_cashflow(price=invalid)
        with self.assertRaises(ValueError): service_cashflow(billable_fraction=1.1)
        with self.assertRaises(ValueError): guarantee_premium(error=1.1)

    def test_nonbillable_attempts_still_cost_money(self):
        model = service_cashflow(billable_fraction=0)
        self.assertEqual(model["net_customer_revenue"], 0)
        self.assertAlmostEqual(model["operating_result_excluding_subsidy"], -20700)
        self.assertIsNone(model["price_for_target_margin"])
        self.assertAlmostEqual(service_cashflow(billable_fraction=.5)["operating_result_excluding_subsidy"], -5700)

    def test_subsidy_does_not_pass_operating_profit_gate(self):
        model = service_cashflow(billable_fraction=0, monthly_subsidy=30_000)
        self.assertGreater(model["cash_result_with_subsidy"], 0)
        self.assertLess(model["operating_result_excluding_subsidy"], 0)

    def test_full_cost_price_and_actor_cash_conservation(self):
        model = service_cashflow(processor_fraction=.03, burn_fraction=.10)
        self.assertAlmostEqual(model["price_for_target_margin"], .0207 / .57)
        self.assertAlmostEqual(model["operating_result_excluding_subsidy"], 5400)
        destinations = sum(model["cost_by_role"].values()) + model["fixed_cost"] + model["processor_paid"] + model["burned"] + model["operating_result_excluding_subsidy"] + model["refunds"]
        self.assertAlmostEqual(destinations, model["gross_billing"])
        self.assertLess(service_cashflow(attempts=10_000)["operating_result_excluding_subsidy"], 0)

    def test_ambiguity_can_remove_challenger_incentive(self):
        self.assertAlmostEqual(challenger_threshold(unresolved_probability=.5), .85)
        self.assertAlmostEqual(challenge_ev(.9, unresolved_probability=.5), 1)
        self.assertLess(challenge_ev(.9, unresolved_probability=.75), 0)
        self.assertIsNone(challenger_threshold(unresolved_probability=1))

    def test_rewards_and_reserve_capture_can_make_farming_profitable(self):
        self.assertLess(coalition_result(), 0)
        self.assertGreater(coalition_result(bonus=85), 0)
        self.assertGreater(coalition_result(bonus=25, reserve_recovery=.8), 0)

    def test_correlated_claims_are_not_consumer_insurance(self):
        model = guarantee_tail(cluster_size=500)
        self.assertEqual(model["reserve_shortfall"], 10000)
        self.assertEqual(model["consumer_compensation"], 0)
        self.assertIsNone(model["estimated_probability"])
        self.assertTrue(model["admission_must_reject"])
        admissible = guarantee_tail(cluster_size=400, reserve=30_000)
        self.assertTrue(admissible["scenario_is_admissible"])
        self.assertEqual(admissible["reserve_shortfall"], 10_000)

    def test_participants_need_positive_net_not_just_a_reward(self):
        model = participant_economics(.002, .003, token_bonus=.002)
        self.assertGreater(model["net_with_assumed_bonus"], 0)
        self.assertFalse(model["sustainable_without_subsidy"])
        self.assertLess(participant_economics(2, 3)["net_without_subsidy"], 0)

    def test_batching_changes_only_the_fee_assumption(self):
        single = solana_fee_assumption()["network_fee_usd_per_query"]
        batch = solana_fee_assumption(queries_per_batch=100)["network_fee_usd_per_query"]
        self.assertAlmostEqual(single, .0015)
        self.assertAlmostEqual(batch, single / 100)
    def test_conservation(self):
        for bounty in [0, 10, 30, 100]:
            model = dispute(bounty)
            for outcome in ["proceeding", "not_proceeding", "unresolved"]:
                self.assertEqual(sum(model[outcome].values()), model["input"])

    def test_incentives(self):
        self.assertEqual(challenge_ev(1, bounty=10), -2)
        self.assertEqual(challenge_ev(1), 18)
        self.assertAlmostEqual(challenge_ev(0.9), 14)
        self.assertAlmostEqual(challenge_ev(0.55), 0)
        self.assertEqual(challenge_ev(1, unresolved_probability=1), -12)
        self.assertLess(challenge_ev(0.9, investigation=40), 0)

    def test_coalition_cannot_recover_reserve(self):
        model = dispute()
        # Even recovering verifier fees and the entire challenger payout,
        # a coalition burning its own author bond loses the restricted reserve.
        self.assertEqual(model["input"] - model["proceeding"]["challenger"]
                         - model["proceeding"]["verifier"], 70)

    def test_query_budget(self):
        self.assertAlmostEqual(query_economics(.02, .003)["monthly_result"], -700)
        self.assertAlmostEqual(query_economics(.03, .003)["monthly_result"], 9300)
        self.assertAlmostEqual(query_economics(.03, .010)["monthly_result"], 2300)
        self.assertGreater(query_economics(.03, .025)["unit_cost"], .03)

    def test_detection_is_not_suppressed_to_show_profit(self):
        model = guarantee_premium()
        self.assertAlmostEqual(model["capital"], 40_000)
        self.assertAlmostEqual(model["premium_subtotal"], 858.3333333333)
        self.assertAlmostEqual(model["premium_with_20pct_cost_markup"], 1030)
        self.assertGreater(guarantee_premium(error=.01)["premium_subtotal"], 700)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--test", action="store_true")
    parser.add_argument("--report", help="Create a new scenario JSON; refuse overwriting history")
    args = parser.parse_args()
    if args.test:
        unittest.main(argv=["economics_pilot"])
    else:
        payload = json.dumps(scenarios(), ensure_ascii=False, indent=2, allow_nan=False)
        if args.report:
            with open(args.report, "x", encoding="utf-8") as report:
                report.write(payload + "\n")
        else:
            print(payload)
