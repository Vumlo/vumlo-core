use sqlx::migrate::Migrator;

use super::*;

struct Fake(&'static Manifest);

impl Module for Fake {
    fn manifest(&self) -> &'static Manifest {
        self.0
    }

    fn register(&self, _: &mut Registrar) {}
}

const fn manifest(id: &'static str, requires: &'static [&'static str]) -> Manifest {
    Manifest {
        id,
        version: "1.0.0",
        requires,
        permissions: &[],
        settings: &[],
        web: false,
    }
}

static A: Manifest = manifest("a", &[]);
static B: Manifest = manifest("b", &["a"]);
static C: Manifest = manifest("c", &["b", "a"]);
static D: Manifest = manifest("d", &[]);
static LONELY: Manifest = manifest("lonely", &["absent"]);
static LEFT: Manifest = manifest("left", &["right"]);
static RIGHT: Manifest = manifest("right", &["left"]);
static BAD: Manifest = manifest("Not-Valid", &[]);

fn ids(order: Vec<&dyn Module>) -> Vec<&'static str> {
    order.iter().map(|m| m.manifest().id).collect()
}

#[test]
fn modules_start_after_what_they_require_and_ties_go_by_id() {
    let (c, d, b, a) = (Fake(&C), Fake(&D), Fake(&B), Fake(&A));
    let order = init_order(&[&c, &d, &b, &a]).unwrap();
    assert_eq!(ids(order), ["a", "b", "c", "d"]);
    // The same set in any input order gives the same answer.
    let order = init_order(&[&a, &b, &c, &d]).unwrap();
    assert_eq!(ids(order), ["a", "b", "c", "d"]);
}

#[test]
fn a_module_that_requires_one_not_compiled_in_stops_the_start_and_names_both() {
    let lonely = Fake(&LONELY);
    let e = init_order(&[&lonely]).err().unwrap();
    assert_eq!(
        e,
        OrderError::Missing {
            module: "lonely".to_owned(),
            missing: "absent".to_owned()
        }
    );
    let message = e.to_string();
    assert!(
        message.contains("lonely") && message.contains("absent"),
        "{message}"
    );
}

#[test]
fn a_cycle_of_requirements_is_refused_and_names_the_modules() {
    let (left, right, a) = (Fake(&LEFT), Fake(&RIGHT), Fake(&A));
    let e = init_order(&[&left, &right, &a]).err().unwrap();
    assert_eq!(
        e,
        OrderError::Cycle(vec!["left".to_owned(), "right".to_owned()])
    );
}

#[test]
fn an_id_that_cannot_be_a_schema_name_and_a_duplicate_are_refused() {
    let (bad, a, again) = (Fake(&BAD), Fake(&A), Fake(&A));
    assert_eq!(
        init_order(&[&bad]).err().unwrap(),
        OrderError::InvalidId("Not-Valid".to_owned())
    );
    assert_eq!(
        init_order(&[&a, &again]).err().unwrap(),
        OrderError::Duplicate("a".to_owned())
    );
}

#[test]
fn a_scoped_migrator_creates_its_schema_and_keeps_its_history_inside_it() {
    let migrator = scoped_migrator("dummy", Migrator::DEFAULT);
    assert_eq!(migrator.table_name, "mod_dummy._sqlx_migrations");
    assert_eq!(
        migrator
            .create_schemas
            .iter()
            .map(|s| s.as_ref())
            .collect::<Vec<_>>(),
        ["mod_dummy"]
    );
}

#[test]
fn the_spec_carries_what_the_configuration_checks() {
    static WEB: Manifest = Manifest {
        web: true,
        ..manifest("w", &["a"])
    };
    let spec = WEB.spec();
    assert_eq!(
        (spec.name, spec.requires, spec.web),
        ("w", &["a"][..], true)
    );
    assert_eq!(WEB.schema(), "mod_w");
}
