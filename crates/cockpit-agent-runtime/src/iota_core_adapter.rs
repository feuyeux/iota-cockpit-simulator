use std::path::{Path, PathBuf};

use iota_core::skill::SkillRegistry;

#[derive(Debug, Clone)]
pub struct IotaCoreAdapter {
    workspace: PathBuf,
}

impl IotaCoreAdapter {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn load_cockpit_skill(&self) -> Result<CockpitSkill, String> {
        let registry = SkillRegistry::load(&self.workspace, &[]);
        let skill = registry
            .get("cockpit-simulation")
            .ok_or_else(|| "cockpit-simulation skill is not registered".to_string())?;
        Ok(CockpitSkill {
            name: skill.metadata.name.clone(),
            version: skill
                .metadata
                .version
                .as_ref()
                .and_then(|value| value.as_str())
                .unwrap_or("1")
                .to_string(),
            body: skill.body.clone(),
            tools: skill
                .metadata
                .execution
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CockpitSkill {
    pub name: String,
    pub version: String,
    pub body: String,
    pub tools: Vec<String>,
}
