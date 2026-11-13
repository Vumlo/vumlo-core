use std::collections::HashMap;

use sqlx::types::Decimal;
use uuid::Uuid;

use super::*;

fn id() -> Uuid {
    Uuid::now_v7()
}

/// Module `top` with sub-module `sub`; `a` and `b` in `top`, `c` in `sub`, `other` is another
/// top-level module with `d`.
struct Course {
    structure: Structure,
    top: Uuid,
    sub: Uuid,
    other: Uuid,
    a: Uuid,
    b: Uuid,
    c: Uuid,
    d: Uuid,
}

fn course() -> Course {
    let (top, sub, other, a, b, c, d) = (id(), id(), id(), id(), id(), id(), id());
    let mut structure = Structure::default();
    structure.add_module(top, None);
    structure.add_module(sub, Some(top));
    structure.add_module(other, None);
    structure.add_item(a, top);
    structure.add_item(b, top);
    structure.add_item(c, sub);
    structure.add_item(d, other);
    Course {
        structure,
        top,
        sub,
        other,
        a,
        b,
        c,
        d,
    }
}

fn done(items: &[Uuid]) -> Facts {
    Facts {
        done_items: items.iter().copied().collect(),
        scores: HashMap::new(),
    }
}

fn rule(node: Node, conditions: Vec<Condition>) -> Rules {
    HashMap::from([(node, conditions)])
}

#[test]
fn an_item_without_a_rule_is_open() {
    let c = course();
    assert!(unmet(Node::Item(c.a), &Rules::new(), &c.structure, &done(&[])).is_empty());
}

#[test]
fn an_item_opens_when_the_item_it_waits_for_is_done() {
    let c = course();
    let condition = Condition::Completed(Node::Item(c.a));
    let rules = rule(Node::Item(c.d), vec![condition.clone()]);
    assert_eq!(
        unmet(Node::Item(c.d), &rules, &c.structure, &done(&[])),
        [condition]
    );
    assert!(unmet(Node::Item(c.d), &rules, &c.structure, &done(&[c.a])).is_empty());
}

#[test]
fn a_module_is_done_when_every_item_in_it_and_in_its_sub_modules_is() {
    let c = course();
    assert!(!is_done(
        Node::Module(c.top),
        &c.structure,
        &done(&[c.a, c.b])
    ));
    assert!(is_done(
        Node::Module(c.top),
        &c.structure,
        &done(&[c.a, c.b, c.c])
    ));
    assert!(is_done(Node::Module(c.sub), &c.structure, &done(&[c.c])));
}

#[test]
fn a_module_with_no_items_is_not_done() {
    let mut c = course();
    let empty = id();
    c.structure.add_module(empty, None);
    assert!(!is_done(Node::Module(empty), &c.structure, &done(&[c.a])));
}

#[test]
fn a_score_condition_needs_the_published_score_to_reach_the_points() {
    let c = course();
    let at_least = Condition::ScoreAtLeast {
        assignment: c.a,
        points: Decimal::new(500, 2),
    };
    let rules = rule(Node::Item(c.d), vec![at_least.clone()]);
    let mut facts = done(&[c.a]);
    // Done is not the same as passed: no score yet, then too low, then enough.
    assert_eq!(
        unmet(Node::Item(c.d), &rules, &c.structure, &facts),
        [at_least]
    );
    facts.scores.insert(c.a, Decimal::new(499, 2));
    assert_eq!(
        unmet(Node::Item(c.d), &rules, &c.structure, &facts).len(),
        1
    );
    facts.scores.insert(c.a, Decimal::new(500, 2));
    assert!(unmet(Node::Item(c.d), &rules, &c.structure, &facts).is_empty());
}

#[test]
fn every_condition_of_a_rule_must_hold() {
    let c = course();
    let rules = rule(
        Node::Item(c.d),
        vec![
            Condition::Completed(Node::Item(c.a)),
            Condition::Completed(Node::Item(c.b)),
        ],
    );
    assert_eq!(
        unmet(Node::Item(c.d), &rules, &c.structure, &done(&[c.a])).len(),
        1
    );
    assert!(unmet(Node::Item(c.d), &rules, &c.structure, &done(&[c.a, c.b])).is_empty());
}

#[test]
fn a_rule_on_a_module_locks_its_items_and_its_sub_modules() {
    let c = course();
    let condition = Condition::Completed(Node::Item(c.d));
    let rules = rule(Node::Module(c.top), vec![condition.clone()]);
    for node in [
        Node::Module(c.top),
        Node::Module(c.sub),
        Node::Item(c.a),
        Node::Item(c.c),
    ] {
        assert_eq!(
            unmet(node, &rules, &c.structure, &done(&[])),
            std::slice::from_ref(&condition)
        );
        assert!(unmet(node, &rules, &c.structure, &done(&[c.d])).is_empty());
    }
    // Another module is not touched.
    assert!(unmet(Node::Item(c.d), &rules, &c.structure, &done(&[])).is_empty());
}

#[test]
fn the_same_condition_from_two_levels_is_listed_once() {
    let c = course();
    let condition = Condition::Completed(Node::Item(c.d));
    let rules = HashMap::from([
        (Node::Module(c.top), vec![condition.clone()]),
        (Node::Item(c.a), vec![condition.clone()]),
    ]);
    assert_eq!(
        unmet(Node::Item(c.a), &rules, &c.structure, &done(&[])),
        [condition]
    );
}

#[test]
fn rules_that_wait_for_each_other_are_a_cycle() {
    let c = course();
    let wait_for = |target: Node| vec![Condition::Completed(target)];
    // a waits for b and b for a.
    let rules = HashMap::from([
        (Node::Item(c.a), wait_for(Node::Item(c.b))),
        (Node::Item(c.b), wait_for(Node::Item(c.a))),
    ]);
    assert!(has_cycle(&c.structure, &rules));
    // An item that waits for itself.
    assert!(has_cycle(
        &c.structure,
        &rule(Node::Item(c.a), wait_for(Node::Item(c.a)))
    ));
    // A chain is not a cycle.
    let chain = HashMap::from([
        (Node::Item(c.a), wait_for(Node::Item(c.b))),
        (Node::Item(c.b), wait_for(Node::Item(c.d))),
    ]);
    assert!(!has_cycle(&c.structure, &chain));
}

#[test]
fn an_item_that_waits_for_its_own_module_is_a_cycle() {
    // The module is done only when the item is, and the item is open only when the module is done.
    let c = course();
    let rules = rule(
        Node::Item(c.a),
        vec![Condition::Completed(Node::Module(c.top))],
    );
    assert!(has_cycle(&c.structure, &rules));
}

#[test]
fn a_module_that_waits_for_something_inside_itself_is_a_cycle() {
    let c = course();
    let rules = rule(
        Node::Module(c.top),
        vec![Condition::Completed(Node::Item(c.c))],
    );
    assert!(has_cycle(&c.structure, &rules));
}

#[test]
fn two_modules_that_wait_for_each_other_are_a_cycle() {
    let c = course();
    let rules = HashMap::from([
        (
            Node::Module(c.top),
            vec![Condition::Completed(Node::Module(c.other))],
        ),
        (
            Node::Module(c.other),
            vec![Condition::Completed(Node::Module(c.top))],
        ),
    ]);
    assert!(has_cycle(&c.structure, &rules));
}

#[test]
fn a_module_that_waits_for_another_module_is_not_a_cycle() {
    let c = course();
    let rules = rule(
        Node::Module(c.other),
        vec![Condition::Completed(Node::Module(c.top))],
    );
    assert!(!has_cycle(&c.structure, &rules));
}
