use std::{
    borrow::{Borrow, BorrowMut},
    collections::HashMap,
    vec,
};

use bollard_stubs::models::{ContainerWaitResponse, ContainerWaitResponseError};
use derive_new::new;
use futures_core::Stream;
use futures_util::{future, StreamExt};
use nonempty::NonEmpty;

use crate::{
    docker::{
        container::{
            CreateContainerConfig, CreateContainerOptions, StartContainerOptions,
            WaitContainerOptions,
        },
        errors::Error,
        Docker,
    },
    BuildResult, BuildRunningState, BuildState, ContainerExitCode, Pipeline, Step, StepName,
    StepResult,
};

pub type CompletedSteps = Vec<(StepName, StepResult)>;
#[derive(Debug, PartialEq, Eq, Clone, new)]
pub struct Build {
    pub pipeline: Pipeline,
    pub state: BuildState,
    pub completed_steps: CompletedSteps,
    #[new(value = "false")]
    pub fail_through: bool,
}

impl Build {
    fn find_completed_steps(
        completed_steps: CompletedSteps,
        step_name_to_match: &StepName,
    ) -> bool {
        completed_steps
            .into_iter()
            .find(|(step_name, res)| step_name.0 == step_name_to_match.0)
            .is_some()
    }
    fn next_step(&self) -> Option<Step> {
        self.pipeline.steps.clone().into_iter().find(|step| {
            Build::find_completed_steps(self.completed_steps.clone(), &step.name) == false
        })
    }
    fn all_steps_succeeded(&self) -> bool {
        self.completed_steps
            .clone()
            .into_iter()
            .all(|(_, res)| res == StepResult::StepSucceeded)
    }
    fn find_depends_on(completed_steps: CompletedSteps, step_depens_on: Vec<StepName>) -> bool {
        step_depens_on
            .into_iter()
            .find(|s| Build::find_completed_steps(completed_steps.clone(), s))
            .is_some()
    }
}

impl Build {
    pub async fn progress(&mut self, conn: &Docker) {
        self.completed_steps.reserve(self.pipeline.steps.len());
        match self.state.clone() {
            BuildState::BuildReady => match self.has_next_step() {
                Ok(step) => {
                    // println!("{:?}", &step.name);
                    // self.state = BuildState::BuildRunning(BuildRunningState { step: step.name })

                    if !self.fail_through {
                        let commands: Vec<String> = step.commands.clone().into();
                        let commands = commands.join(" ");
                        let mut labels = HashMap::new();
                        labels.insert("nova".to_string(), "".to_string());
                        let container = conn
                            .create_container(
                                Some(CreateContainerOptions::new(step.name.clone().0, None)),
                                CreateContainerConfig::new(
                                    step.image.into(),
                                    true,
                                    labels,
                                    vec!["/bin/sh".to_string(), "-c".to_string()],
                                    commands,
                                ),
                            )
                            .await;
                        match container {
                            Ok(container) => {
                                let res = conn
                                    .start_container(
                                        &container.id,
                                        None::<StartContainerOptions<String>>,
                                    )
                                    .await;

                                match res {
                                    Ok(_) => {
                                        self.state = BuildState::BuildRunning(BuildRunningState {
                                            step: step.name.clone(),
                                        })
                                    }
                                    Err(err) => {
                                        println!("{:?}", err);
                                        self.state =
                                            BuildState::BuildFinished(BuildResult::BuildFailed)
                                    }
                                }
                            }
                            Err(err) => {
                                println!("{:?}", err);
                                self.state = BuildState::BuildFinished(BuildResult::BuildFailed)
                            }
                        }
                    } else {
                        self.completed_steps
                            .push((step.name.to_owned(), StepResult::StepSkipped));
                        self.state = BuildState::BuildReady
                    }
                }
                Err(res) => self.state = BuildState::BuildFinished(res),
            },
            BuildState::BuildRunning(state) => {
                let wait = conn.wait_container(
                    &state.step.clone().0,
                    Some(WaitContainerOptions::new("not-running")),
                );
                self.handle_running_state(wait, state.borrow()).await
            }
            BuildState::BuildFinished(_) => todo!(),
        }
    }

    pub fn has_next_step(&self) -> Result<Step, BuildResult> {
        // if self.all_steps_succeeded() {
        match self.next_step() {
            Some(step) => {
                if let Some(ref s) = step.depends_on {
                    // println!("{}", Build::find_depends_on(&self.completed_steps, s));
                    // if !Build::find_depends_on(&self.completed_steps, s) {
                    //     Ok(step)
                    // } else {
                    //     Err(BuildResult::BuildFailed)
                    // }
                    Ok(step)
                } else {
                    Ok(step)
                }
            }
            None => Err(BuildResult::BuildSucceeded),
        }
        // } else {
        //     Err(BuildResult::BuildFailed)
        // }
    }
    async fn handle_running_state<S>(&mut self, wait: S, state: &BuildRunningState)
    where
        S: Stream<Item = Result<ContainerWaitResponse, Error>>,
    {
        wait.for_each(move |s| match s {
            Ok(res) => {
                let exit = ContainerExitCode(res.status_code);
                let result: StepResult = exit.into();
                self.state = BuildState::BuildReady;
                self.completed_steps.push((state.step.to_owned(), result));
                future::ready(())
            }

            Err(Error::DockerContainerWaitError { code, .. }) => {
                self.fail_through = true;
                let exit = ContainerExitCode(code);
                let result: StepResult = exit.into();
                self.state = BuildState::BuildReady;
                self.completed_steps.push((state.step.to_owned(), result));
                future::ready(())
            }
            Err(error) => {
                self.state = BuildState::BuildFinished(BuildResult::BuildFailed);
                println!("{:?}", error);
                future::ready(())
            }
        })
        .await
    }
}
