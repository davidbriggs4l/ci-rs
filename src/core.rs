pub mod build;

use derive_new::new;
use nonempty::NonEmpty;

#[derive(Debug, PartialEq, Eq, Clone, new)]
pub struct Pipeline {
    pub steps: NonEmpty<Step>,
}

#[derive(Debug, PartialEq, Eq, Clone, new)]
pub struct Step {
    pub name: StepName,
    pub commands: NonEmpty<String>,
    pub image: Image,
    pub depends_on: Option<Vec<StepName>>,
}
// impl Step {
//     pub fn new(name: String, image: String, commands: NonEmpty<String>) -> Self {
//         Self {
//             commands,
//             name: StepName(name),
//             image: Image(image),
//         }
//     }
// }

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct StepName(pub String);

impl From<&str> for StepName {
    fn from(value: &str) -> Self {
        StepName(String::from(value))
    }
}
impl From<StepName> for String {
    fn from(value: StepName) -> Self {
        value.0.to_string()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Image(pub String);

impl From<&str> for Image {
    fn from(value: &str) -> Self {
        Image(String::from(value))
    }
}

impl From<Image> for String {
    fn from(value: Image) -> Self {
        value.0.to_string()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum BuildState {
    BuildReady,
    BuildRunning(BuildRunningState),
    BuildFinished(BuildResult),
}
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BuildRunningState {
    pub step: StepName,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum BuildResult {
    BuildSucceeded,
    BuildFailed,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum StepResult {
    StepFailed(ContainerExitCode),
    StepSucceeded,
    StepSkipped,
}
impl From<ContainerExitCode> for StepResult {
    fn from(value: ContainerExitCode) -> Self {
        let v: i64 = value.clone().into();
        if v == 0 {
            StepResult::StepSucceeded
        } else {
            StepResult::StepFailed(ContainerExitCode(v))
        }
    }
}
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ContainerExitCode(pub i64);

impl From<ContainerExitCode> for i64 {
    fn from(value: ContainerExitCode) -> Self {
        value.0
    }
}
