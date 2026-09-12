"""The shipped model's calculated numbers, checked against longhand arithmetic.

Every expected figure here is worked out in the test from the example CSV
inputs and the model's own driver rates, then compared with what the engine
produced. No expected value is copied out of the engine's output, so a test
that fails is a statement about the model or the engine, not about a number
somebody transcribed.

Most figures are exact. Where an asset life divides into additions without
landing on a cent the comparison quantises both sides, and the comment says so.
"""

from decimal import ROUND_HALF_UP, Decimal

from pacioliscube.data import load_into_store
from pacioliscube.evaluate import CellStore, consolidate, evaluate
from pacioliscube.model import load_model

from conftest import EXAMPLES as EXAMPLE_DIR, MODEL_ROOT

MODEL = load_model(MODEL_ROOT)

EXAMPLES = {
    "Drivers": "drivers.csv",
    "Workforce": "workforce.csv",
    "Revenue": "revenue.csv",
    "Capex": "capex.csv",
    "PnL": "pnl-direct.csv",
}

CENTS = Decimal("0.01")

# The shipped data holds Budget only in FY2026-27 and Actual only in FY2025-26,
# so the year and the version travel together as one coordinate prefix.
BUDGET = ("FY2026-27", "Budget")
ACTUAL = ("FY2025-26", "Actual")


def loaded_store() -> CellStore:
    """Every shipped example CSV in one store, before any rule runs."""
    store = CellStore()
    for cube, name in EXAMPLES.items():
        load_into_store(MODEL, cube, EXAMPLE_DIR / name, store)
    return store


# Loading and calculating the whole shipped model costs about half a second, so
# the tests that read it share one result rather than paying that each time.
CALCULATED = evaluate(MODEL, loaded_store())


def cents(value: Decimal) -> Decimal:
    return value.quantize(CENTS, rounding=ROUND_HALF_UP)


def driver(version, measure):
    return CALCULATED.get("Drivers", version + ("Full Year", measure))


def workforce(version, period, entity, cost_centre, role, measure):
    return CALCULATED.get(
        "Workforce", version + (period, entity, cost_centre, role, measure)
    )


def revenue(version, period, entity, cost_centre, measure):
    return CALCULATED.get("Revenue", version + (period, entity, cost_centre, measure))


def capex(version, period, entity, fleet, measure):
    return CALCULATED.get("Capex", version + (period, entity, fleet, measure))


def pnl(version, period, entity, cost_centre, account):
    return CALCULATED.get(
        "PnL", version + (period, entity, cost_centre, account, "Amount")
    )


def pnl_node(version, period, entity, cost_centre, account):
    """A PnL figure read through a coordinate that may name consolidations."""
    return consolidate(
        MODEL, CALCULATED, "PnL", version + (period, entity, cost_centre, account, "Amount")
    )


def test_monthly_base_pay_is_headcount_times_the_annual_rate_over_twelve():
    # CivilCo Earthworks runs 18 operators in July on 125,000 a year each.
    expected = Decimal("18") * Decimal("125000") / Decimal("12")
    assert expected == Decimal("187500")
    assert workforce(BUDGET, "Jul", "CivilCo", "Earthworks", "Operator", "Base Pay") == expected


def test_superannuation_is_the_sg_rate_on_pay_below_the_maximum_contribution_base():
    # The 125,000 operator rate sits under the contribution base, so the whole
    # rate attracts the guarantee. The premise is read from the data, not assumed.
    assert Decimal("125000") < driver(BUDGET, "Maximum Contribution Base")
    expected = Decimal("18") * Decimal("125000") * Decimal("0.12") / Decimal("12")
    assert expected == Decimal("22500")
    assert (
        workforce(BUDGET, "Jul", "CivilCo", "Earthworks", "Operator", "Superannuation Cost")
        == expected
    )


def test_superannuation_is_capped_for_the_role_paid_above_the_contribution_base():
    # CivilCo Drill and Blast has one supervisor on 290,000, above the 270,830
    # base, so the guarantee is charged on the base rather than on the rate.
    assert Decimal("290000") > driver(BUDGET, "Maximum Contribution Base")
    expected = Decimal("1") * Decimal("270830") * Decimal("0.12") / Decimal("12")
    assert expected == Decimal("2708.30")
    assert (
        workforce(BUDGET, "Jul", "CivilCo", "Drill and Blast", "Supervisor", "Superannuation Cost")
        == expected
    )


def test_the_superannuation_cap_bites_because_the_uncapped_charge_is_higher():
    # Without the cap the same supervisor would attract 2,900 a month. A test
    # that only checked the capped figure would pass on a model that had
    # dropped the cap and happened to agree by luck, so state the difference.
    uncapped = Decimal("1") * Decimal("290000") * Decimal("0.12") / Decimal("12")
    assert uncapped == Decimal("2900")
    charged = workforce(
        BUDGET, "Jul", "CivilCo", "Drill and Blast", "Supervisor", "Superannuation Cost"
    )
    assert charged < uncapped
    assert uncapped - charged == Decimal("191.70")


def test_payroll_tax_is_charged_on_pay_grossed_up_by_superannuation():
    # New South Wales taxes wages including superannuation, so the base is the
    # 187,500 of pay plus the 22,500 of guarantee.
    base_pay = Decimal("18") * Decimal("125000") / Decimal("12")
    superannuation = Decimal("18") * Decimal("125000") * Decimal("0.12") / Decimal("12")
    expected = (base_pay + superannuation) * Decimal("0.0545")
    assert expected == Decimal("11445")
    assert (
        workforce(BUDGET, "Jul", "CivilCo", "Earthworks", "Operator", "Payroll Tax Cost")
        == expected
    )


def test_payroll_tax_is_not_charged_on_base_pay_alone():
    # The wrong base is a plausible modelling slip and it lands 1,226.25 short
    # a month on this one role, so the test names the figure it must not equal.
    on_pay_alone = Decimal("18") * Decimal("125000") / Decimal("12") * Decimal("0.0545")
    assert on_pay_alone == Decimal("10218.75")
    charged = workforce(BUDGET, "Jul", "CivilCo", "Earthworks", "Operator", "Payroll Tax Cost")
    assert charged != on_pay_alone
    assert charged - on_pay_alone == Decimal("1226.25")


def test_the_payroll_tax_threshold_credit_reaches_the_designated_group_employer():
    # CivilCo Corporate is the designated group employer. It runs two
    # administrators on 150,000, and it claims the whole group's threshold.
    base_pay = Decimal("2") * Decimal("150000") / Decimal("12")
    superannuation = Decimal("2") * Decimal("150000") * Decimal("0.12") / Decimal("12")
    gross_charge = (base_pay + superannuation) * Decimal("0.0545")
    assert gross_charge == Decimal("1526")
    credit = Decimal("1200000") * Decimal("0.0545") / Decimal("12")
    assert credit == Decimal("5450")
    # The credit is larger than this cost centre's own charge, so the line is
    # negative. That is the intended shape: the group claims the threshold once
    # centrally rather than once per cost centre.
    expected = gross_charge - credit
    assert expected == Decimal("-3924")
    assert pnl(BUDGET, "Jul", "CivilCo", "Corporate", "Payroll Tax") == expected


def test_no_other_cost_centre_of_the_same_entity_receives_the_threshold_credit():
    # CivilCo Earthworks pays tax on the full grossed up figure for both roles.
    operator_pay = Decimal("18") * Decimal("125000") / Decimal("12")
    operator_super = Decimal("18") * Decimal("125000") * Decimal("0.12") / Decimal("12")
    supervisor_pay = Decimal("2") * Decimal("165000") / Decimal("12")
    supervisor_super = Decimal("2") * Decimal("165000") * Decimal("0.12") / Decimal("12")
    expected = (operator_pay + operator_super) * Decimal("0.0545") + (
        supervisor_pay + supervisor_super
    ) * Decimal("0.0545")
    assert expected == Decimal("13123.60")
    assert pnl(BUDGET, "Jul", "CivilCo", "Earthworks", "Payroll Tax") == expected


def test_the_second_entity_corporate_cost_centre_receives_no_threshold_credit():
    # The rule names CivilCo Corporate, not every Corporate cost centre. HaulCo
    # runs no corporate payroll, so its charge is nothing rather than a credit.
    assert pnl(BUDGET, "Jul", "HaulCo", "Corporate", "Payroll Tax") == Decimal("0")


def test_contract_revenue_is_billable_hours_times_the_charge_rate():
    # CivilCo Earthworks plans 5,200 hours in July at 295 an hour.
    expected = Decimal("5200") * Decimal("295")
    assert expected == Decimal("1534000")
    assert revenue(BUDGET, "Jul", "CivilCo", "Earthworks", "Contract Revenue Amount") == expected


def test_fuel_cost_is_litres_times_the_driver_price():
    # 88,000 litres at the 1.85 planned price. The price is a driver, not a
    # literal in the rule, so read it back from the store as well.
    assert driver(BUDGET, "Fuel Price") == Decimal("1.85")
    expected = Decimal("88000") * Decimal("1.85")
    assert expected == Decimal("162800")
    assert revenue(BUDGET, "Jul", "CivilCo", "Earthworks", "Fuel Cost") == expected


def test_depreciation_is_annual_additions_divided_by_the_asset_life_in_months():
    # 2,400,000 of CivilCo excavators over an 84 month life. The division
    # repeats, so both sides are compared at the cent.
    expected = cents(Decimal("2400000") / Decimal("84"))
    assert expected == Decimal("28571.43")
    assert cents(capex(BUDGET, "Jul", "CivilCo", "Excavator", "Depreciation Charge")) == expected


def test_an_asset_class_with_no_life_set_charges_nothing_instead_of_failing():
    # The shipped data gives CivilCo no haul trucks. Buying some without
    # setting a life exercises the safe divide in the rule: the charge is
    # nothing, and evaluating the model does not raise.
    store = loaded_store()
    additions = BUDGET + ("Full Year", "CivilCo", "Haul Truck", "Additions")
    life = BUDGET + ("Full Year", "CivilCo", "Haul Truck", "Asset Life Months")
    store.set("Capex", additions, Decimal("900000"))
    assert not store.has("Capex", life)
    calculated = evaluate(MODEL, store)
    assert calculated.get("Capex", additions) == Decimal("900000")
    charge = calculated.get(
        "Capex", BUDGET + ("Jul", "CivilCo", "Haul Truck", "Depreciation Charge")
    )
    assert charge == Decimal("0")


def test_gross_margin_is_revenue_less_direct_costs_at_a_consolidated_node():
    # CivilCo across all cost centres in July. Only Earthworks and Drill and
    # Blast carry revenue or direct costs; Drill and Blast plans no plant hire.
    revenue_total = (
        Decimal("5200") * Decimal("295")  # Earthworks contract revenue
        + Decimal("180000")  # Earthworks plant hire
        + Decimal("1400") * Decimal("410")  # Drill and Blast contract revenue
    )
    assert revenue_total == Decimal("2288000")
    direct_costs = (
        Decimal("260000")  # Earthworks subcontractors
        + Decimal("95000")  # Drill and Blast subcontractors
        + Decimal("88000") * Decimal("1.85")  # Earthworks fuel
        + Decimal("26000") * Decimal("1.85")  # Drill and Blast fuel
        + Decimal("48000")  # Earthworks consumables
        + Decimal("22000")  # Drill and Blast consumables
    )
    assert direct_costs == Decimal("635900")
    expected = revenue_total - direct_costs
    assert expected == Decimal("1652100")
    assert pnl_node(BUDGET, "Jul", "CivilCo", "All Cost Centres", "Gross Margin") == expected
    # The two sides the hierarchy subtracts, read back on their own.
    assert pnl_node(BUDGET, "Jul", "CivilCo", "All Cost Centres", "Revenue") == revenue_total
    assert pnl_node(BUDGET, "Jul", "CivilCo", "All Cost Centres", "Direct Costs") == direct_costs


def test_group_depreciation_is_the_years_additions_spread_over_each_asset_life():
    # Five asset classes across the two entities, each month charging a
    # twelfth of nothing more than additions over life, times twelve months.
    monthly = (
        Decimal("2400000") / Decimal("84")  # CivilCo excavators
        + Decimal("1500000") / Decimal("84")  # CivilCo dozers
        + Decimal("360000") / Decimal("48")  # CivilCo light vehicles
        + Decimal("5400000") / Decimal("96")  # HaulCo haul trucks
        + Decimal("240000") / Decimal("48")  # HaulCo light vehicles
    )
    # Two of those lives leave a repeating decimal, so compare at the cent.
    expected = cents(monthly * 12)
    assert expected == Decimal("1382142.86")
    charged = pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "Depreciation")
    assert cents(charged) == expected


def test_ebit_is_ebitda_less_depreciation_for_the_group_across_the_full_year():
    ebit = pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "EBIT")
    ebitda = pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "EBITDA")
    depreciation = pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "Depreciation")
    # Depreciation carries a repeating decimal from the asset life division, and
    # the engine sums it at a different point on each side of this identity, so
    # both sides are compared at the cent.
    assert cents(ebit) == cents(ebitda - depreciation)
    # Guard against the identity holding because every term is nothing.
    assert depreciation > Decimal("0")
    assert ebit < ebitda


def test_the_budget_revenue_total_beats_the_actual_revenue_total():
    # Three cost centres carry revenue and the shipped data repeats the same
    # hours, rates and plant hire in every month of both years.
    budget_month = (
        Decimal("5200") * Decimal("295")  # CivilCo Earthworks contract
        + Decimal("180000")  # CivilCo Earthworks plant hire
        + Decimal("1400") * Decimal("410")  # CivilCo Drill and Blast contract
        + Decimal("7800") * Decimal("255")  # HaulCo Haulage contract
        + Decimal("120000")  # HaulCo Haulage plant hire
    )
    actual_month = (
        Decimal("4888") * Decimal("295")
        + Decimal("169200")
        + Decimal("1316") * Decimal("410")
        + Decimal("7332") * Decimal("255")
        + Decimal("112800")
    )
    budget_year = budget_month * 12
    actual_year = actual_month * 12
    assert budget_year == Decimal("52764000")
    assert actual_year == Decimal("49598160")
    assert pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "Revenue") == budget_year
    assert pnl_node(ACTUAL, "FY", "Group", "All Cost Centres", "Revenue") == actual_year
    # Every line in that sum is higher in the budget, so the total must be too.
    assert budget_year > actual_year


def test_the_pnl_wages_line_equals_workforce_base_pay_summed_over_roles():
    operator = Decimal("18") * Decimal("125000") / Decimal("12")
    supervisor = Decimal("2") * Decimal("165000") / Decimal("12")
    expected = operator + supervisor
    assert expected == Decimal("215000")
    assert pnl(BUDGET, "Jul", "CivilCo", "Earthworks", "Wages and Salaries") == expected
    # The same figure taken from the source cube role by role, which is what
    # the cross cube rule is meant to be reporting.
    roles = ("Operator", "Supervisor", "Maintenance", "Administration")
    from_workforce = sum(
        (
            workforce(BUDGET, "Jul", "CivilCo", "Earthworks", role, "Base Pay")
            for role in roles
        ),
        Decimal("0"),
    )
    assert from_workforce == expected


def test_the_contribution_base_cap_binds_in_the_actual_year_at_its_own_figure():
    # Each year carries its own maximum contribution base, so a test pinned only
    # to the budget year cannot see the actual year's figure change. The same
    # supervisor on 290,000 is capped at the FY2025-26 base of 250,000, which is
    # the 30,000 concessional cap divided by the 12 per cent guarantee rate.
    base = driver(ACTUAL, "Maximum Contribution Base")
    assert base == Decimal("250000")
    assert Decimal("290000") > base
    expected = Decimal("1") * base * driver(ACTUAL, "SG Rate") / Decimal("12")
    assert expected == Decimal("2500")
    assert (
        workforce(ACTUAL, "Jul", "CivilCo", "Drill and Blast", "Supervisor", "Superannuation Cost")
        == expected
    )


def test_each_year_caps_superannuation_at_its_own_contribution_base():
    # The two years differ, so the same role on the same pay draws a different
    # charge. A single shared cap would make these equal.
    budget_charge = workforce(
        BUDGET, "Jul", "CivilCo", "Drill and Blast", "Supervisor", "Superannuation Cost"
    )
    actual_charge = workforce(
        ACTUAL, "Jul", "CivilCo", "Drill and Blast", "Supervisor", "Superannuation Cost"
    )
    assert budget_charge != actual_charge
    assert budget_charge - actual_charge == Decimal("208.30")
