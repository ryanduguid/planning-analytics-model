//! The shipped model's calculated numbers, checked against longhand arithmetic,
//! and consolidation conservation checked from leaf cells.
//!
//! Every expected figure is worked out in the test from the example CSV inputs
//! and the model's own driver rates. No expected value is copied out of the
//! engine's output.

mod common;

use std::sync::OnceLock;

use common::*;
use pacioliscube::data::load_into_store;
use pacioliscube::decimal::Decimal;
use pacioliscube::error::Error;
use pacioliscube::evaluate::{CellStore, consolidate_many, evaluate};
use pacioliscube::model::{Model, load_model};
use pacioliscube::pyjson::{self, Json};
use pacioliscube::pypath::PyPath;

const EXAMPLES: [(&str, &str); 5] = [
    ("Drivers", "drivers.csv"),
    ("Workforce", "workforce.csv"),
    ("Revenue", "revenue.csv"),
    ("Capex", "capex.csv"),
    ("PnL", "pnl-direct.csv"),
];

const BUDGET: [&str; 2] = ["FY2026-27", "Budget"];
const ACTUAL: [&str; 2] = ["FY2025-26", "Actual"];

struct Shipped {
    model: Model,
    calculated: CellStore,
}

fn loaded_store(model: &Model) -> CellStore {
    let mut store = CellStore::new();
    for (cube, name) in EXAMPLES {
        load_into_store(
            model,
            cube,
            &PyPath::new(&s(&examples().join(name))),
            &mut store,
        )
        .unwrap();
    }
    store
}

/// Loading and calculating the whole shipped model is shared by every test.
fn shipped() -> &'static Shipped {
    static SHIPPED: OnceLock<Shipped> = OnceLock::new();
    SHIPPED.get_or_init(|| {
        big_stack(|| {
            let model = load_model(&PyPath::new(&s(&model_root()))).unwrap();
            let calculated = evaluate(&model, &loaded_store(&model)).unwrap();
            Shipped { model, calculated }
        })
    })
}

fn at(version: [&str; 2], rest: &[&str]) -> Vec<String> {
    let mut out = coord(&version);
    out.extend(coord(rest));
    out
}

fn get(cube: &str, coordinate: Vec<String>) -> Decimal {
    shipped().calculated.get(cube, &coordinate)
}

fn driver(version: [&str; 2], measure: &str) -> Decimal {
    get("Drivers", at(version, &["Full Year", measure]))
}

fn workforce(
    version: [&str; 2],
    period: &str,
    entity: &str,
    cost_centre: &str,
    role: &str,
    measure: &str,
) -> Decimal {
    get(
        "Workforce",
        at(version, &[period, entity, cost_centre, role, measure]),
    )
}

fn revenue(
    version: [&str; 2],
    period: &str,
    entity: &str,
    cost_centre: &str,
    measure: &str,
) -> Decimal {
    get(
        "Revenue",
        at(version, &[period, entity, cost_centre, measure]),
    )
}

fn capex(version: [&str; 2], period: &str, entity: &str, fleet: &str, measure: &str) -> Decimal {
    get("Capex", at(version, &[period, entity, fleet, measure]))
}

fn pnl(
    version: [&str; 2],
    period: &str,
    entity: &str,
    cost_centre: &str,
    account: &str,
) -> Decimal {
    get(
        "PnL",
        at(version, &[period, entity, cost_centre, account, "Amount"]),
    )
}

fn nodes(
    model: &Model,
    store: &CellStore,
    cube: &str,
    coordinates: Vec<Vec<String>>,
) -> Vec<Decimal> {
    let cells: Vec<(String, Vec<String>)> = coordinates
        .into_iter()
        .map(|c| (cube.to_string(), c))
        .collect();
    let mut values = Vec::new();
    consolidate_many(model, store, &cells, |_, v| values.push(v)).unwrap();
    values
}

fn node(cube: &str, coordinate: Vec<String>) -> Decimal {
    let shipped = shipped();
    nodes(&shipped.model, &shipped.calculated, cube, vec![coordinate]).remove(0)
}

fn pnl_node(
    version: [&str; 2],
    period: &str,
    entity: &str,
    cost_centre: &str,
    account: &str,
) -> Decimal {
    node(
        "PnL",
        at(version, &[period, entity, cost_centre, account, "Amount"]),
    )
}

fn n(text: &str) -> Decimal {
    d(text)
}

#[test]
fn monthly_base_pay_is_headcount_times_the_annual_rate_over_twelve() {
    let expected = n("18").mul_(&n("125000")).div_(&n("12"));
    assert_eq!(expected, n("187500"));
    assert_eq!(
        workforce(
            BUDGET,
            "Jul",
            "CivilCo",
            "Earthworks",
            "Operator",
            "Base Pay"
        ),
        expected
    );
}

#[test]
fn superannuation_is_the_sg_rate_on_pay_below_the_maximum_contribution_base() {
    assert!(n("125000") < driver(BUDGET, "Maximum Contribution Base"));
    let expected = n("18").mul_(&n("125000")).mul_(&n("0.12")).div_(&n("12"));
    assert_eq!(expected, n("22500"));
    assert_eq!(
        workforce(
            BUDGET,
            "Jul",
            "CivilCo",
            "Earthworks",
            "Operator",
            "Superannuation Cost"
        ),
        expected
    );
}

#[test]
fn superannuation_is_capped_for_the_role_paid_above_the_contribution_base() {
    assert!(n("290000") > driver(BUDGET, "Maximum Contribution Base"));
    let expected = n("1").mul_(&n("270830")).mul_(&n("0.12")).div_(&n("12"));
    assert_eq!(expected, n("2708.30"));
    let charged = workforce(
        BUDGET,
        "Jul",
        "CivilCo",
        "Drill and Blast",
        "Supervisor",
        "Superannuation Cost",
    );
    assert_eq!(charged, expected);
    let uncapped = n("1").mul_(&n("290000")).mul_(&n("0.12")).div_(&n("12"));
    assert_eq!(uncapped, n("2900"));
    assert!(charged < uncapped);
    assert_eq!(uncapped.sub_(&charged), n("191.70"));
}

#[test]
fn payroll_tax_is_charged_on_pay_grossed_up_by_superannuation() {
    let base_pay = n("18").mul_(&n("125000")).div_(&n("12"));
    let superannuation = n("18").mul_(&n("125000")).mul_(&n("0.12")).div_(&n("12"));
    let expected = base_pay.add_(&superannuation).mul_(&n("0.0545"));
    assert_eq!(expected, n("11445"));
    let charged = workforce(
        BUDGET,
        "Jul",
        "CivilCo",
        "Earthworks",
        "Operator",
        "Payroll Tax Cost",
    );
    assert_eq!(charged, expected);
    let on_pay_alone = n("18").mul_(&n("125000")).div_(&n("12")).mul_(&n("0.0545"));
    assert_eq!(on_pay_alone, n("10218.75"));
    assert_ne!(charged, on_pay_alone);
    assert_eq!(charged.sub_(&on_pay_alone), n("1226.25"));
}

#[test]
fn the_payroll_tax_threshold_credit_reaches_the_designated_group_employer() {
    let base_pay = n("2").mul_(&n("150000")).div_(&n("12"));
    let superannuation = n("2").mul_(&n("150000")).mul_(&n("0.12")).div_(&n("12"));
    let gross_charge = base_pay.add_(&superannuation).mul_(&n("0.0545"));
    assert_eq!(gross_charge, n("1526"));
    // July's return covers 31 of the year's 365 days.
    let credit = n("1200000")
        .mul_(&n("0.0545"))
        .div_(&n("365").div_(&n("31")));
    assert_eq!(credit.round2(), n("5554.52"));
    let expected = gross_charge.sub_(&credit);
    assert_eq!(expected.round2(), n("-4028.52"));
    assert_eq!(
        pnl(BUDGET, "Jul", "CivilCo", "Corporate", "Payroll Tax"),
        expected
    );
}

#[test]
fn the_threshold_credit_follows_the_days_in_each_month_and_adds_to_the_year() {
    let february = n("1526").sub_(
        &n("1200000")
            .mul_(&n("0.0545"))
            .div_(&n("365").div_(&n("28"))),
    );
    assert_eq!(february.round2(), n("-3490.99"));
    assert_eq!(
        pnl(BUDGET, "Feb", "CivilCo", "Corporate", "Payroll Tax"),
        february
    );
    let year = n("12").mul_(&n("1526")).sub_(&n("65400"));
    assert!(close(
        &pnl_node(BUDGET, "FY", "CivilCo", "Corporate", "Payroll Tax"),
        &year,
        "0.000000001"
    ));
}

#[test]
fn a_leap_year_spreads_the_credit_over_366_days() {
    let model = &shipped().model;
    let (july, total) = {
        let model_ref: &'static Model = model;
        big_stack(move || {
            let mut store = loaded_store(model_ref);
            store.set("Drivers", &at(BUDGET, &["Feb", "Days"]), n("29"));
            store.set("Drivers", &at(BUDGET, &["Full Year", "Days"]), n("366"));
            let leap = evaluate(model_ref, &store).unwrap();
            let july = leap.get(
                "PnL",
                &at(
                    BUDGET,
                    &["Jul", "CivilCo", "Corporate", "Payroll Tax", "Amount"],
                ),
            );
            let total = nodes(
                model_ref,
                &leap,
                "PnL",
                vec![at(
                    BUDGET,
                    &["FY", "CivilCo", "Corporate", "Payroll Tax", "Amount"],
                )],
            )
            .remove(0);
            (july, total)
        })
    };
    let expected = n("1526").sub_(&n("65400").div_(&n("366").div_(&n("31"))));
    assert_eq!(expected.round2(), n("-4013.34"));
    assert_eq!(july, expected);
    let year = n("12").mul_(&n("1526")).sub_(&n("65400"));
    assert!(close(&total, &year, "0.000000001"));
}

#[test]
fn a_thresholded_year_missing_its_days_stops_the_calculation() {
    let model: &'static Model = &shipped().model;
    for period in ["Feb", "Full Year"] {
        let error = big_stack(move || {
            let mut store = loaded_store(model);
            store.set("Drivers", &at(BUDGET, &[period, "Days"]), n("0"));
            evaluate(model, &store).unwrap_err()
        });
        assert!(
            matches!(&error, Error::Evaluation(m) if m.contains("division by zero")),
            "{error:?}"
        );
    }
}

#[test]
fn full_year_takes_its_workforce_payroll_tax_and_no_threshold_share() {
    let workforce = node(
        "Workforce",
        at(
            BUDGET,
            &[
                "Full Year",
                "CivilCo",
                "Corporate",
                "All Roles",
                "Payroll Tax Cost",
            ],
        ),
    );
    assert_eq!(
        pnl(BUDGET, "Full Year", "CivilCo", "Corporate", "Payroll Tax"),
        workforce
    );
}

fn days_in(year: i64, month: u32) -> i64 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[test]
fn the_shipped_days_are_the_calendar_days_of_each_thresholded_year() {
    let months = [
        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec", "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    ];
    for version in [BUDGET, ACTUAL] {
        assert!(!driver(version, "Payroll Tax Threshold").is_zero());
        let first: i64 = version[0][2..6].parse().unwrap();
        for (number, period) in months.iter().enumerate() {
            let (year, month) = if number < 6 {
                (first, number as u32 + 7)
            } else {
                (first + 1, number as u32 - 5)
            };
            let days = Decimal::from_int(days_in(year, month));
            assert_eq!(
                get("Drivers", at(version, &[period, "Days"])),
                days,
                "{version:?} {period}"
            );
        }
        let leap = days_in(first + 1, 2) == 29;
        let year_days = Decimal::from_int(if leap { 366 } else { 365 });
        assert_eq!(
            get("Drivers", at(version, &["Full Year", "Days"])),
            year_days
        );
        assert_eq!(node("Drivers", at(version, &["FY", "Days"])), year_days);
    }
}

#[test]
fn only_the_designated_employer_receives_the_threshold_credit() {
    let operator_pay = n("18").mul_(&n("125000")).div_(&n("12"));
    let operator_super = n("18").mul_(&n("125000")).mul_(&n("0.12")).div_(&n("12"));
    let supervisor_pay = n("2").mul_(&n("165000")).div_(&n("12"));
    let supervisor_super = n("2").mul_(&n("165000")).mul_(&n("0.12")).div_(&n("12"));
    let expected = operator_pay
        .add_(&operator_super)
        .mul_(&n("0.0545"))
        .add_(&supervisor_pay.add_(&supervisor_super).mul_(&n("0.0545")));
    assert_eq!(expected, n("13123.60"));
    assert_eq!(
        pnl(BUDGET, "Jul", "CivilCo", "Earthworks", "Payroll Tax"),
        expected
    );
    assert_eq!(
        pnl(BUDGET, "Jul", "HaulCo", "Corporate", "Payroll Tax"),
        n("0")
    );
}

#[test]
fn contract_revenue_is_billable_hours_times_the_charge_rate() {
    // CivilCo Earthworks plans 5,200 hours in July at 295 an hour.
    let expected = n("5200").mul_(&n("295"));
    assert_eq!(expected, n("1534000"));
    assert_eq!(
        revenue(
            BUDGET,
            "Jul",
            "CivilCo",
            "Earthworks",
            "Contract Revenue Amount"
        ),
        expected
    );
}

#[test]
fn fuel_cost_is_litres_times_the_driver_price() {
    // The price is a driver, not a literal in the rule, so read it back as well.
    assert_eq!(driver(BUDGET, "Fuel Price"), n("1.85"));
    let expected = n("88000").mul_(&n("1.85"));
    assert_eq!(expected, n("162800"));
    assert_eq!(
        revenue(BUDGET, "Jul", "CivilCo", "Earthworks", "Fuel Cost"),
        expected
    );
}

#[test]
fn depreciation_is_annual_additions_divided_by_the_asset_life_in_months() {
    // The division repeats, so both sides are compared at the cent.
    let expected = n("2400000").div_(&n("84")).cents();
    assert_eq!(expected, n("28571.43"));
    assert_eq!(
        capex(BUDGET, "Jul", "CivilCo", "Excavator", "Depreciation Charge").cents(),
        expected
    );
}

#[test]
fn an_asset_class_with_no_life_set_charges_nothing_instead_of_failing() {
    let model: &'static Model = &shipped().model;
    let (additions, charge) = big_stack(move || {
        let mut store = loaded_store(model);
        let additions = at(BUDGET, &["Full Year", "CivilCo", "Haul Truck", "Additions"]);
        let life = at(
            BUDGET,
            &["Full Year", "CivilCo", "Haul Truck", "Asset Life Months"],
        );
        store.set("Capex", &additions, n("900000"));
        assert!(!store.has("Capex", &life));
        let calculated = evaluate(model, &store).unwrap();
        (
            calculated.get("Capex", &additions),
            calculated.get(
                "Capex",
                &at(
                    BUDGET,
                    &["Jul", "CivilCo", "Haul Truck", "Depreciation Charge"],
                ),
            ),
        )
    });
    assert_eq!(additions, n("900000"));
    assert_eq!(charge, n("0"));
}

#[test]
fn gross_margin_is_revenue_less_direct_costs_at_a_consolidated_node() {
    let revenue_total = n("5200")
        .mul_(&n("295"))
        .add_(&n("180000"))
        .add_(&n("1400").mul_(&n("410")));
    assert_eq!(revenue_total, n("2288000"));
    let direct_costs = n("260000")
        .add_(&n("95000"))
        .add_(&n("88000").mul_(&n("1.85")))
        .add_(&n("26000").mul_(&n("1.85")))
        .add_(&n("48000"))
        .add_(&n("22000"));
    assert_eq!(direct_costs, n("635900"));
    let expected = revenue_total.sub_(&direct_costs);
    assert_eq!(expected, n("1652100"));
    assert_eq!(
        pnl_node(BUDGET, "Jul", "CivilCo", "All Cost Centres", "Gross Margin"),
        expected
    );
    assert_eq!(
        pnl_node(BUDGET, "Jul", "CivilCo", "All Cost Centres", "Revenue"),
        revenue_total
    );
    assert_eq!(
        pnl_node(BUDGET, "Jul", "CivilCo", "All Cost Centres", "Direct Costs"),
        direct_costs
    );
}

#[test]
fn group_depreciation_and_ebit() {
    let monthly = n("2400000")
        .div_(&n("84"))
        .add_(&n("1500000").div_(&n("84")))
        .add_(&n("360000").div_(&n("48")))
        .add_(&n("5400000").div_(&n("96")))
        .add_(&n("240000").div_(&n("48")));
    let expected = monthly.mul_(&n("12")).cents();
    assert_eq!(expected, n("1382142.86"));
    let depreciation = pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "Depreciation");
    assert_eq!(depreciation.cents(), expected);
    let ebit = pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "EBIT");
    let ebitda = pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "EBITDA");
    assert_eq!(ebit.cents(), ebitda.sub_(&depreciation).cents());
    assert!(depreciation > n("0"));
    assert!(ebit < ebitda);
}

#[test]
fn the_budget_revenue_total_beats_the_actual_revenue_total() {
    let budget_month = n("5200")
        .mul_(&n("295"))
        .add_(&n("180000"))
        .add_(&n("1400").mul_(&n("410")))
        .add_(&n("7800").mul_(&n("255")))
        .add_(&n("120000"));
    let actual_month = n("4888")
        .mul_(&n("295"))
        .add_(&n("169200"))
        .add_(&n("1316").mul_(&n("410")))
        .add_(&n("7332").mul_(&n("255")))
        .add_(&n("112800"));
    let budget_year = budget_month.mul_(&n("12"));
    let actual_year = actual_month.mul_(&n("12"));
    assert_eq!(budget_year, n("52764000"));
    assert_eq!(actual_year, n("49598160"));
    assert_eq!(
        pnl_node(BUDGET, "FY", "Group", "All Cost Centres", "Revenue"),
        budget_year
    );
    assert_eq!(
        pnl_node(ACTUAL, "FY", "Group", "All Cost Centres", "Revenue"),
        actual_year
    );
    assert!(budget_year > actual_year);
}

#[test]
fn the_pnl_wages_line_equals_workforce_base_pay_summed_over_roles() {
    let expected = n("18")
        .mul_(&n("125000"))
        .div_(&n("12"))
        .add_(&n("2").mul_(&n("165000")).div_(&n("12")));
    assert_eq!(expected, n("215000"));
    assert_eq!(
        pnl(BUDGET, "Jul", "CivilCo", "Earthworks", "Wages and Salaries"),
        expected
    );
    let mut from_workforce = Decimal::zero();
    for role in ["Operator", "Supervisor", "Maintenance", "Administration"] {
        from_workforce = from_workforce.add_(&workforce(
            BUDGET,
            "Jul",
            "CivilCo",
            "Earthworks",
            role,
            "Base Pay",
        ));
    }
    assert_eq!(from_workforce, expected);
}

#[test]
fn each_year_caps_superannuation_at_its_own_contribution_base() {
    let base = driver(ACTUAL, "Maximum Contribution Base");
    assert_eq!(base, n("250000"));
    assert!(n("290000") > base);
    let expected = n("1")
        .mul_(&base)
        .mul_(&driver(ACTUAL, "SG Rate"))
        .div_(&n("12"));
    assert_eq!(expected, n("2500"));
    let actual_charge = workforce(
        ACTUAL,
        "Jul",
        "CivilCo",
        "Drill and Blast",
        "Supervisor",
        "Superannuation Cost",
    );
    assert_eq!(actual_charge, expected);
    let budget_charge = workforce(
        BUDGET,
        "Jul",
        "CivilCo",
        "Drill and Blast",
        "Supervisor",
        "Superannuation Cost",
    );
    assert_ne!(budget_charge, actual_charge);
    assert_eq!(budget_charge.sub_(&actual_charge), n("208.30"));
}

// ---------------------------------------------------------------- conservation

const TOLERANCE: &str = "0.000000001";
const ENTITIES: [&str; 2] = ["CivilCo", "HaulCo"];
const COST_CENTRES: [&str; 6] = [
    "Earthworks",
    "Drill and Blast",
    "Haulage",
    "Workshop",
    "Site Admin",
    "Corporate",
];
const MONTHS: [&str; 12] = [
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec", "Jan", "Feb", "Mar", "Apr", "May", "Jun",
];
const QUARTERS: [(&str, [&str; 3]); 4] = [
    ("Q1", ["Jul", "Aug", "Sep"]),
    ("Q2", ["Oct", "Nov", "Dec"]),
    ("Q3", ["Jan", "Feb", "Mar"]),
    ("Q4", ["Apr", "May", "Jun"]),
];
const REVENUE_ACCOUNTS: [&str; 2] = ["Contract Revenue", "Plant Hire Revenue"];
const COST_ACCOUNTS: [&str; 11] = [
    "Subcontractor Costs",
    "Fuel",
    "Consumables",
    "Wages and Salaries",
    "Superannuation",
    "Payroll Tax",
    "Repairs and Maintenance",
    "Insurance",
    "Site Overheads",
    "Corporate Overheads",
    "Depreciation",
];

fn leaf_accounts() -> Vec<&'static str> {
    REVENUE_ACCOUNTS
        .iter()
        .chain(&COST_ACCOUNTS)
        .copied()
        .collect()
}

fn leaf_sum(
    version: [&str; 2],
    months: &[&str],
    entities: &[&str],
    cost_centres: &[&str],
    accounts: &[&str],
) -> Decimal {
    let mut total = Decimal::zero();
    for m in months {
        for e in entities {
            for cc in cost_centres {
                for a in accounts {
                    total = total.add_(&pnl(version, m, e, cc, a));
                }
            }
        }
    }
    total
}

fn pnl_nodes(coordinates: Vec<Vec<String>>) -> Vec<Decimal> {
    let shipped = shipped();
    let coordinates = coordinates
        .into_iter()
        .map(|mut c| {
            c.push("Amount".into());
            c
        })
        .collect();
    nodes(&shipped.model, &shipped.calculated, "PnL", coordinates)
}

#[test]
fn the_model_has_enough_calculated_leaves_for_these_checks_to_mean_something() {
    let accounts = leaf_accounts();
    let mut non_zero = Vec::new();
    for version in [BUDGET, ACTUAL] {
        for m in MONTHS {
            for e in ENTITIES {
                for cc in COST_CENTRES {
                    for a in &accounts {
                        if !pnl(version, m, e, cc, a).is_zero() {
                            non_zero.push([m, e, cc, a]);
                        }
                    }
                }
            }
        }
    }
    assert!(non_zero.len() > 500);
    for (position, members) in [&MONTHS[..], &ENTITIES[..], &COST_CENTRES[..], &accounts[..]]
        .iter()
        .enumerate()
    {
        for member in members.iter() {
            assert!(
                non_zero.iter().any(|cell| cell[position] == *member),
                "{member}"
            );
        }
    }
    let limit = d(TOLERANCE);
    for version in [BUDGET, ACTUAL] {
        for line_item in &accounts {
            let total = leaf_sum(version, &MONTHS, &ENTITIES, &COST_CENTRES, &[line_item]);
            assert!(
                total > limit || total < limit.neg().unwrap(),
                "{version:?} {line_item}"
            );
        }
        assert!(
            leaf_sum(
                version,
                &MONTHS,
                &ENTITIES,
                &COST_CENTRES,
                &REVENUE_ACCOUNTS
            ) > Decimal::zero()
        );
    }
}

#[test]
fn group_and_entities_are_the_sums_of_their_leaves() {
    let accounts = leaf_accounts();
    for version in [BUDGET, ACTUAL] {
        let mut cells = Vec::new();
        for m in MONTHS {
            for a in &accounts {
                cells.push((m, *a));
            }
        }
        let totals = pnl_nodes(
            cells
                .iter()
                .map(|(m, a)| at(version, &[m, "Group", "All Cost Centres", a]))
                .collect(),
        );
        for ((m, a), total) in cells.iter().zip(&totals) {
            assert!(
                close(
                    total,
                    &leaf_sum(version, &[m], &ENTITIES, &COST_CENTRES, &[a]),
                    TOLERANCE
                ),
                "{m} {a}"
            );
        }
        let mut cells = Vec::new();
        for m in MONTHS {
            for e in ENTITIES {
                for a in &accounts {
                    cells.push((m, e, *a));
                }
            }
        }
        let totals = pnl_nodes(
            cells
                .iter()
                .map(|(m, e, a)| at(version, &[m, e, "All Cost Centres", a]))
                .collect(),
        );
        for ((m, e, a), total) in cells.iter().zip(&totals) {
            assert!(
                close(
                    total,
                    &leaf_sum(version, &[m], &[e], &COST_CENTRES, &[a]),
                    TOLERANCE
                ),
                "{m} {e} {a}"
            );
        }
    }
}

#[test]
fn quarters_and_the_year_are_the_sums_of_their_months() {
    let accounts = leaf_accounts();
    for version in [BUDGET, ACTUAL] {
        let mut leaves = Vec::new();
        for e in ENTITIES {
            for cc in COST_CENTRES {
                for a in &accounts {
                    leaves.push((e, cc, *a));
                }
            }
        }
        let periods = ["Q1", "Q2", "Q3", "Q4", "FY"];
        let mut coordinates = Vec::new();
        for (e, cc, a) in &leaves {
            for p in periods {
                coordinates.push(at(version, &[p, e, cc, a]));
            }
        }
        let mut values = pnl_nodes(coordinates).into_iter();
        for (e, cc, a) in &leaves {
            let by_month = |m: &str| pnl(version, m, e, cc, a);
            for (quarter, members) in QUARTERS {
                let mut sum = Decimal::zero();
                for m in members {
                    sum = sum.add_(&by_month(m));
                }
                assert!(
                    close(&values.next().unwrap(), &sum, TOLERANCE),
                    "{quarter} {e} {cc} {a}"
                );
            }
            let mut year = Decimal::zero();
            for m in MONTHS {
                year = year.add_(&by_month(m));
            }
            assert!(
                close(&values.next().unwrap(), &year, TOLERANCE),
                "FY {e} {cc} {a}"
            );
        }
    }
}

#[test]
fn ebit_is_revenue_leaves_less_every_cost_leaf() {
    for version in [BUDGET, ACTUAL] {
        for entity in ["CivilCo", "HaulCo", "Group"] {
            let members: Vec<&str> = if entity == "Group" {
                ENTITIES.to_vec()
            } else {
                vec![entity]
            };
            let ebit = pnl_nodes(vec![at(
                version,
                &["FY", entity, "All Cost Centres", "EBIT"],
            )])
            .remove(0);
            let revenue = leaf_sum(version, &MONTHS, &members, &COST_CENTRES, &REVENUE_ACCOUNTS);
            let costs = leaf_sum(version, &MONTHS, &members, &COST_CENTRES, &COST_ACCOUNTS);
            assert!(
                close(&ebit, &revenue.sub_(&costs), TOLERANCE),
                "{version:?} {entity}"
            );
        }
    }
}

// The pending native comparison, bound to real inputs and independent arithmetic.

fn comparison() -> &'static Json {
    static COMPARISON: OnceLock<Json> = OnceLock::new();
    COMPARISON.get_or_init(|| {
        let path = repo().join("docs").join("native-comparison-manifest.json");
        pyjson::loads(&std::fs::read_to_string(path).unwrap()).unwrap()
    })
}

fn case(index: usize) -> &'static Json {
    match comparison().get("cases") {
        Some(Json::Array(cases)) => &cases[index],
        _ => panic!("the manifest lists its cases"),
    }
}

fn field<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key)
        .and_then(Json::as_str)
        .unwrap_or_else(|| panic!("{key} is a string"))
}

fn names(json: &Json, key: &str) -> Vec<String> {
    match json.get(key) {
        Some(Json::Array(items)) => items
            .iter()
            .map(|item| item.as_str().unwrap().to_string())
            .collect(),
        _ => panic!("{key} is a list"),
    }
}

/// Runs the existing independent calculation a baseline case names.
fn independent(test: &str) {
    match test {
        "test_monthly_base_pay_is_headcount_times_the_annual_rate_over_twelve" => {
            monthly_base_pay_is_headcount_times_the_annual_rate_over_twelve()
        }
        "test_superannuation_is_the_sg_rate_on_pay_below_the_maximum_contribution_base" => {
            superannuation_is_the_sg_rate_on_pay_below_the_maximum_contribution_base()
        }
        "test_superannuation_is_capped_for_the_role_paid_above_the_contribution_base" => {
            superannuation_is_capped_for_the_role_paid_above_the_contribution_base()
        }
        "test_payroll_tax_is_charged_on_pay_grossed_up_by_superannuation" => {
            payroll_tax_is_charged_on_pay_grossed_up_by_superannuation()
        }
        "test_contract_revenue_is_billable_hours_times_the_charge_rate" => {
            contract_revenue_is_billable_hours_times_the_charge_rate()
        }
        "test_depreciation_is_annual_additions_divided_by_the_asset_life_in_months" => {
            depreciation_is_annual_additions_divided_by_the_asset_life_in_months()
        }
        "test_the_payroll_tax_threshold_credit_reaches_the_designated_group_employer" => {
            the_payroll_tax_threshold_credit_reaches_the_designated_group_employer()
        }
        "test_gross_margin_is_revenue_less_direct_costs_at_a_consolidated_node" => {
            gross_margin_is_revenue_less_direct_costs_at_a_consolidated_node()
        }
        other => panic!("no independent calculation named {other}"),
    }
}

/// The example inputs copied to a scratch directory, less the case's removed rows.
fn altered_inputs(case: &Json) -> tempfile::TempDir {
    let scratch = tempdir();
    for entry in std::fs::read_dir(examples()).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, scratch.path().join(path.file_name().unwrap())).unwrap();
    }
    let removal = case.get("remove_rows").unwrap();
    let Some(Json::Object(conditions)) = removal.get("where") else {
        panic!("remove_rows names its conditions")
    };
    let path = scratch.path().join(field(removal, "file"));
    let text = std::fs::read_to_string(&path).unwrap();
    // The shipped CSVs carry no quoted fields, so a line splits on its commas.
    assert!(!text.contains('"'));
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let rows: Vec<&str> = lines.collect();
    let retained: Vec<&str> = rows
        .iter()
        .copied()
        .filter(|line| {
            let values: Vec<&str> = line.split(',').collect();
            !conditions.iter().all(|(column, wanted)| {
                let at = header.iter().position(|h| h == column).unwrap();
                values[at] == wanted.as_str().unwrap()
            })
        })
        .collect();
    assert!(
        retained.len() < rows.len(),
        "a variant must actually remove its intended synthetic rows"
    );
    let mut written = header.join(",");
    written.push('\n');
    for line in retained {
        written.push_str(line);
        written.push('\n');
    }
    std::fs::write(&path, written).unwrap();
    scratch
}

#[test]
fn the_manifest_has_eight_baseline_cases_two_variants_and_no_native_claim() {
    let comparison = comparison();
    assert_eq!(
        comparison.get("schema_version"),
        Some(&pyjson::loads("1").unwrap())
    );
    let Some(Json::Array(cases)) = comparison.get("cases") else {
        panic!("the manifest lists its cases")
    };
    let ids: Vec<&str> = cases.iter().map(|c| field(c, "id")).collect();
    let expected: Vec<String> = (1..=10).map(|n| format!("TM1-{n:03}")).collect();
    assert_eq!(ids, expected);
    let baselines = cases.iter().filter(|c| field(c, "variant") == "baseline");
    assert_eq!(baselines.count(), 8);
    let pending = r#"{"status": "pending", "server_version": null, "server_build": null,
        "operator": null, "run_date": null, "raw_exports": []}"#;
    assert_eq!(
        comparison.get("native_evidence"),
        Some(&pyjson::loads(pending).unwrap())
    );
    let tolerance = r#"{"currency_unit": "0.01", "maximum_raw_difference": "0.005"}"#;
    assert_eq!(
        comparison.get("tolerance"),
        Some(&pyjson::loads(tolerance).unwrap())
    );
    assert_eq!(
        case(7).get("native_zero_suppression"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        case(9).get("fresh_feeders_before_check_feeders"),
        Some(&Json::Bool(true))
    );
}

#[test]
fn the_manifest_hashes_cover_every_model_file_and_all_five_inputs() {
    use sha2::{Digest, Sha256};
    fn walk(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&model_root(), &mut files);
    walk(&examples(), &mut files);
    let actual: std::collections::BTreeMap<String, String> = files
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(repo()).unwrap();
            let name = relative
                .iter()
                .map(|part| part.to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            let digest = Sha256::digest(std::fs::read(path).unwrap());
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            (name, hex)
        })
        .collect();
    let Some(Json::Object(recorded)) = comparison().get("input_hashes") else {
        panic!("the manifest records its input hashes")
    };
    let recorded: std::collections::BTreeMap<String, String> = recorded
        .iter()
        .map(|(name, hash)| (name.clone(), hash.as_str().unwrap().to_string()))
        .collect();
    assert_eq!(recorded, actual);
    assert_eq!(
        actual.keys().filter(|n| n.starts_with("examples/")).count(),
        5
    );
}

#[test]
fn each_baseline_case_agrees_with_the_existing_independent_calculation() {
    for index in 0..8 {
        let case = case(index);
        independent(field(case, "independent_test"));
        let shipped = shipped();
        let actual = nodes(
            &shipped.model,
            &shipped.calculated,
            field(case, "cube"),
            vec![names(case, "coordinates")],
        )
        .remove(0);
        assert!(
            close(&actual, &d(field(case, "expected")), "0.005"),
            "{}",
            field(case, "id")
        );
    }
}

#[test]
fn missing_february_days_is_an_offline_error_not_a_native_observation() {
    let case = case(8);
    assert_eq!(field(case, "expected"), "N/A");
    assert_eq!(case.get("offline_exit"), Some(&pyjson::loads("3").unwrap()));
    let data = altered_inputs(case);
    let cell = format!(
        "{}:{}",
        field(case, "cube"),
        names(case, "coordinates").join(",")
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .args([
            "calculate",
            &s(&model_root()),
            "--data",
            &s(data.path()),
            "--cell",
            &cell,
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("division by zero"));
}

#[test]
fn removing_corporate_workforce_preserves_its_credit_in_the_group() {
    let case = case(9);
    let data = altered_inputs(case);
    let directory = s(data.path());
    let model: &'static Model = &shipped().model;
    let cells = vec![names(case, "coordinates"), names(case, "group_coordinates")];
    let cube = field(case, "cube").to_string();
    let values = big_stack(move || {
        let store = pacioliscube::data::load_data(model, &PyPath::new(&directory)).unwrap();
        let calculated = evaluate(model, &store).unwrap();
        nodes(model, &calculated, &cube, cells)
    });
    let (corporate, group) = (&values[0], &values[1]);
    let expected_credit = n("1200000").mul_(&n("0.0545")).neg().unwrap();
    assert!(close(&d(field(case, "expected")), &expected_credit, "0"));
    assert!(close(corporate, &expected_credit, "0.005"));
    let original_group = node(field(case, "cube"), names(case, "group_coordinates"));
    // Removing two 150,000 salaries removes their annual gross payroll-tax charge,
    // while the threshold credit remains in both Corporate and Group.
    let removed_charge = n("2")
        .mul_(&n("150000"))
        .mul_(&n("1.12"))
        .mul_(&n("0.0545"));
    assert!(close(
        group,
        &original_group.sub_(&removed_charge),
        "0.000000001"
    ));
}
