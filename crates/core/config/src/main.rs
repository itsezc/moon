use moon_config::{
    GlobalProjectConfig, ProjectConfig, TemplateConfig, TemplateFrontmatterConfig, ToolchainConfig,
    WorkspaceConfig,
};
use schemars::schema_for;
use std::fs;

fn main() {
    // Generate JSON schemas derived from our structs
    let project_schema = schema_for!(ProjectConfig);
    let global_project_schema = schema_for!(GlobalProjectConfig);
    let template_schema = schema_for!(TemplateConfig);
    let template_frontmatter_schema = schema_for!(TemplateFrontmatterConfig);
    let toolchain_schema = schema_for!(ToolchainConfig);
    let workspace_schema = schema_for!(WorkspaceConfig);

    fs::write(
        "website/static/schemas/project.json",
        serde_json::to_string_pretty(&project_schema).unwrap(),
    )
    .unwrap();

    fs::write(
        "website/static/schemas/global-project.json",
        serde_json::to_string_pretty(&global_project_schema).unwrap(),
    )
    .unwrap();

    fs::write(
        "website/static/schemas/template.json",
        serde_json::to_string_pretty(&template_schema).unwrap(),
    )
    .unwrap();

    fs::write(
        "website/static/schemas/template-frontmatter.json",
        serde_json::to_string_pretty(&template_frontmatter_schema).unwrap(),
    )
    .unwrap();

    fs::write(
        "website/static/schemas/toolchain.json",
        serde_json::to_string_pretty(&toolchain_schema).unwrap(),
    )
    .unwrap();

    fs::write(
        "website/static/schemas/workspace.json",
        serde_json::to_string_pretty(&workspace_schema).unwrap(),
    )
    .unwrap();
}
