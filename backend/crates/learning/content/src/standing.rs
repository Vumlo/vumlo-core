//! Completion and access rules over the course tree (ADR-0092). `rules` decides, from plain data,
//! what keeps a node closed; this file loads that data for one course and puts the answer on the
//! nodes of the tree, and checks and stores a rule that a teacher sets.
//!
//! It authorizes nothing: the service that calls it has already decided who is asking, and whether
//! the asker is staff (who see the rule and are never locked) or a student (who see where they
//! stand and never the rule's internals beyond what stands in their way).

use std::collections::HashMap;

use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::model::{
    Access, ConditionInput, ConditionKind, ContentModule, ItemSummary, Module, Standing, Unmet,
};
use super::repo;
use super::rules::{self, Condition, Node, Rules, Structure};
use super::service::ContentError;
use crate::points;

/// The most conditions one rule has.
const MAX_CONDITIONS: usize = 10;

/// The items and modules of a tree, and what each is called, as the reader of the tree sees them.
fn structure_of(tree: &[ContentModule]) -> (Structure, HashMap<Node, String>) {
    let mut structure = Structure::default();
    let mut titles = HashMap::new();
    for top in tree {
        register(&mut structure, &mut titles, &top.module, None, &top.items);
        for sub in &top.submodules {
            register(
                &mut structure,
                &mut titles,
                &sub.module,
                Some(top.module.id),
                &sub.items,
            );
        }
    }
    (structure, titles)
}

/// Adds a module and its items to the structure and their titles to `titles`.
fn register(
    structure: &mut Structure,
    titles: &mut HashMap<Node, String>,
    module: &Module,
    parent: Option<Uuid>,
    items: &[ItemSummary],
) {
    structure.add_module(module.id, parent);
    titles.insert(Node::Module(module.id), module.title.clone());
    for item in items {
        structure.add_item(item.id, module.id);
        titles.insert(Node::Item(item.id), item.title.clone());
    }
}

/// Where a student stands on every module and item of `tree`, which must be the tree they can
/// see: a node they cannot see does not count towards a module being done, and is not named when
/// it keeps something locked.
pub async fn student_standings(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_id: Uuid,
    tree: &[ContentModule],
) -> Result<HashMap<Node, Standing>, sqlx::Error> {
    let (structure, titles) = structure_of(tree);
    let rules = repo::rules_of_course(conn, course_id).await?;
    let facts = repo::facts(conn, course_id, student_id).await?;
    Ok(structure
        .nodes()
        .map(|node| {
            let locked = rules::unmet(node, &rules, &structure, &facts)
                .into_iter()
                .map(|condition| Unmet {
                    title: titles.get(&condition.target()).cloned(),
                    condition,
                })
                .collect();
            let done = rules::is_done(node, &structure, &facts);
            (node, Standing { done, locked })
        })
        .collect())
}

/// Sets `access` on every module and item of the tree from what `access_of` says about it.
pub fn attach(tree: &mut [ContentModule], access_of: impl Fn(Node) -> Access) {
    fn module(module: &mut Module, access_of: &impl Fn(Node) -> Access) {
        module.access = access_of(Node::Module(module.id));
    }
    fn items(items: &mut [ItemSummary], access_of: &impl Fn(Node) -> Access) {
        for item in items {
            item.access = access_of(Node::Item(item.id));
        }
    }
    for top in tree {
        module(&mut top.module, &access_of);
        items(&mut top.items, &access_of);
        for sub in &mut top.submodules {
            module(&mut sub.module, &access_of);
            items(&mut sub.items, &access_of);
        }
    }
}

/// What staff are told about a node: the rule that locks it.
pub fn staff_access(rules: &Rules, node: Node) -> Access {
    Access {
        rule: rules.get(&node).cloned().unwrap_or_default(),
        standing: None,
    }
}

/// What a student is told about a node: where they stand.
pub fn student_access(standings: &HashMap<Node, Standing>, node: Node) -> Access {
    Access {
        rule: Vec::new(),
        standing: standings.get(&node).cloned(),
    }
}

/// Makes `inputs` the rule of `owner` after checking them against the course, and returns the
/// conditions stored. `tree` is the whole tree of the course, drafts included. An empty `inputs`
/// removes the rule.
///
/// A condition names something in this course other than the node it locks. A score condition
/// names an assignment and asks for more than nothing and at most what the assignment is worth.
/// The rules together must not make a circle (see [`rules::has_cycle`]). The writers of one
/// course's rules take turns, so two rules that are fine alone cannot close a circle together.
pub async fn set_rule(
    conn: &mut PgConnection,
    course_id: Uuid,
    owner: Node,
    inputs: &[ConditionInput],
    tree: &[ContentModule],
) -> Result<Vec<Condition>, ContentError> {
    if inputs.len() > MAX_CONDITIONS {
        return Err(ContentError::Rule {
            field: "access_rule.all".to_owned(),
            message: "has at most 10 conditions",
        });
    }
    repo::lock_rules(conn, course_id).await?;
    let mut conditions = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        conditions.push(check(conn, course_id, owner, index, input).await?);
    }
    let mut all = repo::rules_of_course(conn, course_id).await?;
    if conditions.is_empty() {
        all.remove(&owner);
    } else {
        all.insert(owner, conditions.clone());
    }
    if rules::has_cycle(&structure_of(tree).0, &all) {
        return Err(ContentError::Rule {
            field: "access_rule".to_owned(),
            message: "would lock part of the course until something that waits for it is done",
        });
    }
    repo::replace_rule(conn, course_id, owner, &conditions).await?;
    Ok(conditions)
}

/// One condition of a client's rule, checked: `index` is its place in the list, for the error.
async fn check(
    conn: &mut PgConnection,
    course_id: Uuid,
    owner: Node,
    index: usize,
    input: &ConditionInput,
) -> Result<Condition, ContentError> {
    let field = |name: &str| format!("access_rule.all[{index}].{name}");
    let invalid = |name: &str, message: &'static str| ContentError::Rule {
        field: field(name),
        message,
    };
    match (input.kind, input.item_id, input.module_id, &input.points) {
        (ConditionKind::Completed, item, module, None) => {
            let target = match (item, module) {
                (Some(item), None) => Node::Item(item),
                (None, Some(module)) => Node::Module(module),
                _ => return Err(invalid("item_id", "name either an item or a module")),
            };
            if target == owner {
                return Err(invalid("item_id", "cannot wait for itself"));
            }
            if !in_course(conn, course_id, target).await? {
                let name = if item.is_some() {
                    "item_id"
                } else {
                    "module_id"
                };
                return Err(invalid(name, "is not in this course"));
            }
            Ok(Condition::Completed(target))
        }
        (ConditionKind::Completed, ..) => {
            Err(invalid("points", "a completed condition has no points"))
        }
        (ConditionKind::ScoreAtLeast, Some(assignment), None, Some(points)) => {
            if Node::Item(assignment) == owner {
                return Err(invalid("item_id", "cannot wait for itself"));
            }
            let Some(item) = repo::find_item(conn, assignment).await? else {
                return Err(invalid("item_id", "is not in this course"));
            };
            if item.item.course_id != course_id {
                return Err(invalid("item_id", "is not in this course"));
            }
            let Some(worth) = item.item.assignment.as_ref().map(|a| a.max_points.value()) else {
                return Err(invalid("item_id", "is not an assignment"));
            };
            let points =
                parse_points(points, worth).map_err(|message| invalid("points", message))?;
            Ok(Condition::ScoreAtLeast { assignment, points })
        }
        (ConditionKind::ScoreAtLeast, ..) => Err(invalid(
            "item_id",
            "a score condition names an assignment and the points",
        )),
    }
}

/// A number of points above 0, with at most two decimals, that the assignment can give.
fn parse_points(raw: &str, worth: Decimal) -> Result<Decimal, &'static str> {
    match points::parse_decimal(raw) {
        Some(points) if points > Decimal::ZERO && points.scale() <= 2 && points <= worth => {
            Ok(points)
        }
        _ => Err("must be above 0 and at most what the assignment is worth"),
    }
}

/// Whether the item or the module is part of the course.
async fn in_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    node: Node,
) -> Result<bool, sqlx::Error> {
    Ok(match node {
        Node::Item(id) => repo::find_item(conn, id)
            .await?
            .is_some_and(|row| row.item.course_id == course_id),
        Node::Module(id) => repo::find_module(conn, id)
            .await?
            .is_some_and(|row| row.module.course_id == course_id),
    })
}
