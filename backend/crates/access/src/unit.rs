//! The vocabulary of org units (ADR-0094): the roles a person holds on a unit, the abilities an
//! admin gives those roles, and the grant `teaching` hands to `authorize`. `org` stores the text.

use uuid::Uuid;

use common::{Role, text_enum};

/// A person's role on a unit; it covers the unit and every unit below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitRole {
    Manager,
    Viewer,
}
text_enum!(UnitRole { Manager => "manager", Viewer => "viewer" });

/// A switch an admin gives a unit role. Viewing is implicit to any grant and has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitAbility {
    ViewGrades,
    EditContent,
    Grade,
    ManageRoster,
    ManageCourses,
    ManageUnits,
}
text_enum!(UnitAbility {
    ViewGrades => "view_grades",
    EditContent => "edit_content",
    Grade => "grade",
    ManageRoster => "manage_roster",
    ManageCourses => "manage_courses",
    ManageUnits => "manage_units",
});

impl UnitAbility {
    pub const ALL: [UnitAbility; 6] = [
        Self::ViewGrades,
        Self::EditContent,
        Self::Grade,
        Self::ManageRoster,
        Self::ManageCourses,
        Self::ManageUnits,
    ];

    fn bit(self) -> u8 {
        match self {
            Self::ViewGrades => 1,
            Self::EditContent => 1 << 1,
            Self::Grade => 1 << 2,
            Self::ManageRoster => 1 << 3,
            Self::ManageCourses => 1 << 4,
            Self::ManageUnits => 1 << 5,
        }
    }
}

/// The union of the abilities of every unit role that covers one unit. Holding one at all is the
/// implicit ability to view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitGrant {
    pub unit_id: Uuid,
    abilities: u8,
}

impl UnitGrant {
    pub fn new(unit_id: Uuid, abilities: impl IntoIterator<Item = UnitAbility>) -> Self {
        Self {
            unit_id,
            abilities: abilities
                .into_iter()
                .fold(0, |set, ability| set | ability.bit()),
        }
    }

    /// The grant made of ability names as stored. A name that is no ability is an error, never a
    /// smaller grant: dropping it would silently take a right away (or, were the rule ever
    /// inverted, give one).
    pub fn from_names(unit_id: Uuid, names: &[String]) -> Result<Self, String> {
        let abilities = names
            .iter()
            .map(|name| name.parse::<UnitAbility>())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(unit_id, abilities))
    }

    /// Every ability: an admin's view of a unit.
    pub fn full(unit_id: Uuid) -> Self {
        Self::new(unit_id, UnitAbility::ALL)
    }

    pub fn has(self, ability: UnitAbility) -> bool {
        self.abilities & ability.bit() != 0
    }

    /// The abilities held, in the order of [`UnitAbility::ALL`].
    pub fn abilities(self) -> Vec<UnitAbility> {
        UnitAbility::ALL
            .into_iter()
            .filter(|ability| self.has(*ability))
            .collect()
    }
}

/// May a user with this institution role hold a unit role? Teachers and admins; a student never.
pub fn unit_role_allowed(institution: Role) -> bool {
    matches!(institution, Role::Teacher | Role::Admin)
}
