//! Access rules and completion (ADR-0092): what a student must have done before a module or an
//! item opens. Pure: no database and no clock. The service loads the structure of the course, the
//! rules and what the student has done, and asks the functions here what is still missing.
//!
//! A node (an item or a module) can carry a rule, a list of conditions that must all hold. A rule
//! on a module applies to everything in it, and a rule on a top-level module to its sub-modules
//! too, so what stands between a student and an item is the conditions of the item, of its module
//! and of that module's parent that do not hold yet.

use std::collections::{HashMap, HashSet};

use sqlx::types::Decimal;
use uuid::Uuid;

/// An item or a module: what a rule can lock and what a condition can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Node {
    Item(Uuid),
    Module(Uuid),
}

/// One thing a student must have done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// The item is done, or every item in the module is.
    Completed(Node),
    /// The published score of the assignment is at least `points`.
    ScoreAtLeast { assignment: Uuid, points: Decimal },
}

impl Condition {
    /// The node the condition is about.
    pub fn target(&self) -> Node {
        match self {
            Self::Completed(node) => *node,
            Self::ScoreAtLeast { assignment, .. } => Node::Item(*assignment),
        }
    }
}

/// The conditions of each node of a course that has a rule. All of a node's conditions must hold.
pub type Rules = HashMap<Node, Vec<Condition>>;

/// How the items and modules of a course hang together. Sub-modules are one level deep.
#[derive(Debug, Default, Clone)]
pub struct Structure {
    /// The module each item is in.
    module_of: HashMap<Uuid, Uuid>,
    /// The top-level module of each sub-module.
    parent_of: HashMap<Uuid, Uuid>,
    modules: HashSet<Uuid>,
}

impl Structure {
    pub fn add_module(&mut self, id: Uuid, parent: Option<Uuid>) {
        self.modules.insert(id);
        if let Some(parent) = parent {
            self.parent_of.insert(id, parent);
        }
    }

    pub fn add_item(&mut self, id: Uuid, module: Uuid) {
        self.module_of.insert(id, module);
    }

    /// The items in the module and in its sub-modules.
    pub fn items_under(&self, module: Uuid) -> Vec<Uuid> {
        self.module_of
            .iter()
            .filter(|&(_, &home)| home == module || self.parent_of.get(&home) == Some(&module))
            .map(|(&item, _)| item)
            .collect()
    }

    /// The nodes whose rules apply to `node`: the node itself, its module and that module's parent.
    fn chain(&self, node: Node) -> Vec<Node> {
        let mut chain = vec![node];
        // The module the node is in; a module is in its own, which is already in the chain.
        let module = match node {
            Node::Item(item) => {
                let module = self.module_of.get(&item).copied();
                chain.extend(module.map(Node::Module));
                module
            }
            Node::Module(module) => Some(module),
        };
        chain.extend(
            module
                .and_then(|module| self.parent_of.get(&module))
                .map(|&parent| Node::Module(parent)),
        );
        chain
    }

    /// Every item and module of the course.
    pub fn nodes(&self) -> impl Iterator<Item = Node> + '_ {
        self.module_of
            .keys()
            .map(|&item| Node::Item(item))
            .chain(self.modules.iter().map(|&module| Node::Module(module)))
    }
}

/// What a student has done in a course.
#[derive(Debug, Default, Clone)]
pub struct Facts {
    /// Items that are done: marked by the student, or an assignment that was submitted or graded.
    pub done_items: HashSet<Uuid>,
    /// The published score of each assignment the student has one for.
    pub scores: HashMap<Uuid, Decimal>,
}

/// Whether the student has done the item, or every item in the module. A module with no items
/// that the student can see is not done: nothing was finished.
pub fn is_done(node: Node, structure: &Structure, facts: &Facts) -> bool {
    match node {
        Node::Item(item) => facts.done_items.contains(&item),
        Node::Module(module) => {
            let items = structure.items_under(module);
            !items.is_empty() && items.iter().all(|item| facts.done_items.contains(item))
        }
    }
}

fn is_met(condition: &Condition, structure: &Structure, facts: &Facts) -> bool {
    match condition {
        Condition::Completed(node) => is_done(*node, structure, facts),
        Condition::ScoreAtLeast { assignment, points } => facts
            .scores
            .get(assignment)
            .is_some_and(|score| score >= points),
    }
}

/// The conditions that still keep `node` locked: its own, those of its module and those of the
/// module's parent that do not hold. Empty when the node is open.
pub fn unmet(node: Node, rules: &Rules, structure: &Structure, facts: &Facts) -> Vec<Condition> {
    let mut missing: Vec<Condition> = Vec::new();
    for owner in structure.chain(node) {
        for condition in rules.get(&owner).into_iter().flatten() {
            if !is_met(condition, structure, facts) && !missing.contains(condition) {
                missing.push(condition.clone());
            }
        }
    }
    missing
}

/// Whether the rules can never be satisfied because a node waits, directly or through the module
/// it is in, for something that waits for it. A module waits for its items, and an item waits for
/// the rules of the modules around it, so a rule on an item that names its own module is a circle.
pub fn has_cycle(structure: &Structure, rules: &Rules) -> bool {
    let mut marks = HashMap::new();
    structure
        .nodes()
        .chain(rules.keys().copied())
        .any(|start| circles_back(start, structure, rules, &mut marks))
}

/// Where the depth-first search of [`has_cycle`] stands with a node.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// The node is on the path being followed: reaching it again closes a circle.
    OnPath,
    /// Everything the node waits for was followed and leads to no circle.
    Clear,
}

/// Whether following what `node` waits for leads back to a node already on the path.
fn circles_back(
    node: Node,
    structure: &Structure,
    rules: &Rules,
    marks: &mut HashMap<Node, Mark>,
) -> bool {
    match marks.get(&node) {
        Some(Mark::OnPath) => return true,
        Some(Mark::Clear) => return false,
        None => {}
    }
    marks.insert(node, Mark::OnPath);
    if waits_for(node, structure, rules)
        .into_iter()
        .any(|next| circles_back(next, structure, rules, marks))
    {
        return true;
    }
    marks.insert(node, Mark::Clear);
    false
}

/// What must happen before `node` can be done or opened: the targets of every rule that applies to
/// it, and for a module also its items.
fn waits_for(node: Node, structure: &Structure, rules: &Rules) -> Vec<Node> {
    let mut targets: Vec<Node> = structure
        .chain(node)
        .into_iter()
        .flat_map(|owner| {
            rules
                .get(&owner)
                .into_iter()
                .flatten()
                .map(Condition::target)
        })
        .collect();
    if let Node::Module(module) = node {
        targets.extend(structure.items_under(module).into_iter().map(Node::Item));
    }
    targets
}

#[cfg(test)]
mod tests;
