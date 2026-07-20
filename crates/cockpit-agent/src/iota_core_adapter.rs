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
        self.load_cockpit_skill_localized("en")
    }

    /// Load the cockpit skill, using a language-localized body when available.
    ///
    /// The base skill (name/version/tools + English body) always comes from the
    /// `SKILL.md` resource via the [`SkillRegistry`]. When `language` is a
    /// Chinese tag and a sibling `SKILL.zh.md` body resource exists next to it,
    /// that body replaces the English one, keeping prompts resource-driven and
    /// bilingual without duplicating the skill's metadata/tool contract.
    pub fn load_cockpit_skill_localized(&self, language: &str) -> Result<CockpitSkill, String> {
        let registry = SkillRegistry::load(&self.workspace, &[]);
        let skill = registry
            .get("cockpit-world")
            .ok_or_else(|| "cockpit-world skill is not registered".to_string())?;
        let mut body = skill.body.clone();
        if matches!(language, "zh" | "zh-CN" | "zh-Hans")
            && let Some(dir) = skill.path.parent()
        {
            let localized = dir.join("SKILL.zh.md");
            if let Ok(content) = std::fs::read_to_string(&localized) {
                body = strip_frontmatter(&content);
            }
        }
        Ok(CockpitSkill {
            name: skill.metadata.name.clone(),
            version: skill
                .metadata
                .version
                .as_ref()
                .and_then(|value| value.as_str())
                .unwrap_or("1")
                .to_string(),
            body,
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

/// Strip a leading YAML frontmatter block (delimited by `---` lines) from a
/// localized skill body file, returning the body text. A file with no
/// frontmatter is returned trimmed as-is, so a plain-markdown localization
/// asset works too.
fn strip_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---") {
        // Find the closing `---` delimiter of the frontmatter block.
        if let Some(end) = rest.find("\n---") {
            let after = &rest[end + "\n---".len()..];
            // Skip to the end of the delimiter line.
            let body = after.split_once('\n').map(|(_, body)| body).unwrap_or("");
            return body.trim().to_string();
        }
    }
    trimmed.trim().to_string()
}
