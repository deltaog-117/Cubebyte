//! What a blueprint holds.

use thiserror::Error;

use crate::crafting::custom::{CustomRecipes, name_is_valid};
use crate::logic::model::Rules;
use crate::scripting::program::Program;

/// Why a blueprint cannot be made.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlueprintError {
    #[error("a blueprint needs a rule, a recipe or a build")]
    Empty,
    #[error("a build's name is 1 to 16 letters, digits, - or _ (lower case)")]
    BadName,
}

/// A program that places blocks, with the name it was shared under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    name: String,
    program: Program,
}

impl Build {
    /// # Errors
    ///
    /// Returns [`BlueprintError::BadName`] when `name` is not a valid recipe-style name.
    pub fn try_new(name: &str, program: Program) -> Result<Self, BlueprintError> {
        if !name_is_valid(name) {
            return Err(BlueprintError::BadName);
        }
        Ok(Self {
            name: name.to_owned(),
            program,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn program(&self) -> &Program {
        &self.program
    }
}

/// Something worth sharing: at least one rule, recipe or build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blueprint {
    rules: Rules,
    recipes: CustomRecipes,
    build: Option<Build>,
}

impl Blueprint {
    /// # Errors
    ///
    /// Returns [`BlueprintError::Empty`] when all three parts are empty.
    pub fn try_new(
        rules: Rules,
        recipes: CustomRecipes,
        build: Option<Build>,
    ) -> Result<Self, BlueprintError> {
        if rules.is_empty() && recipes.is_empty() && build.is_none() {
            return Err(BlueprintError::Empty);
        }
        Ok(Self {
            rules,
            recipes,
            build,
        })
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    pub fn recipes(&self) -> &CustomRecipes {
        &self.recipes
    }

    pub fn build(&self) -> Option<&Build> {
        self.build.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::text::parse;

    #[test]
    fn an_empty_blueprint_is_refused() {
        assert_eq!(
            Blueprint::try_new(Rules::default(), CustomRecipes::default(), None),
            Err(BlueprintError::Empty)
        );
    }

    #[test]
    fn a_build_alone_is_a_blueprint() {
        let program = parse("build stone 0 0 0\n").unwrap();
        let build = Build::try_new("tower", program).unwrap();
        assert!(
            Blueprint::try_new(Rules::default(), CustomRecipes::default(), Some(build)).is_ok()
        );
    }

    #[test]
    fn a_build_name_follows_the_recipe_name_rule() {
        let program = parse("build stone 0 0 0\n").unwrap();
        assert_eq!(
            Build::try_new("Bad Name", program),
            Err(BlueprintError::BadName)
        );
    }
}
