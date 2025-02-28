mod detect;
mod download;
mod execute;
mod install;
mod resolve;
mod verify;

use proto_core::{Describable, Proto, Tool};
// use resolve::NDMVersionDist;
use std::path::PathBuf;

#[derive(Debug)]
pub enum NodeDependencyManagerType {
    Npm,
    Pnpm,
    Yarn,
}

impl NodeDependencyManagerType {
    pub fn get_package_name(&self) -> String {
        match self {
            NodeDependencyManagerType::Npm => "npm".into(),
            NodeDependencyManagerType::Pnpm => "pnpm".into(),
            NodeDependencyManagerType::Yarn => "yarn".into(),
        }
    }
}

#[derive(Debug)]
pub struct NodeDependencyManager {
    pub bin_path: Option<PathBuf>,
    // pub dist: Option<NDMVersionDist>,
    pub install_dir: PathBuf,
    pub log_target: String,
    pub temp_dir: PathBuf,
    pub type_of: NodeDependencyManagerType,
    pub version: String,
}

impl NodeDependencyManager {
    pub fn new(proto: &Proto, type_of: NodeDependencyManagerType, version: Option<&str>) -> Self {
        let package_name = type_of.get_package_name();

        NodeDependencyManager {
            bin_path: None,
            // dist: None,
            install_dir: proto.tools_dir.join(&package_name),
            log_target: format!("proto:tool:{}", &package_name),
            temp_dir: proto.temp_dir.join(&package_name),
            type_of,
            version: version.unwrap_or("latest").into(),
        }
    }

    // pub fn get_dist(&self) -> &NDMVersionDist {
    //     self.dist
    //         .as_ref()
    //         .expect("Distribution info not defined for node dependency manager!")
    // }
}

impl Describable<'_> for NodeDependencyManager {
    fn get_log_target(&self) -> &str {
        &self.log_target
    }

    fn get_name(&self) -> String {
        self.type_of.get_package_name()
    }
}

impl Tool<'_> for NodeDependencyManager {}
