use core::{build::*, *};
use docker::{
    container::{CreateContainerConfig, CreateContainerOptions, StartContainerOptions},
    Docker,
};

use nonempty::nonempty;
use std::{collections::HashMap, time::Duration};

mod core;
mod docker;

#[tokio::main]
async fn main() {
    let conn = Docker::connect_with_unix_defaults().unwrap();

    let steps = nonempty![
        //
        Step::new(
            "build".into(),
            nonempty!["dat".to_string()],
            "ubuntu:20.04".into(),
            None
        ),
        Step::new(
            "test".into(),
            // nonempty!["sleep 10s && date".to_string()],
            nonempty!["ps".to_string()],
            "ubuntu:20.04".into(),
            Some(vec!["test".into()])
        ),
        Step::new(
            "deploy".into(),
            // nonempty!["sleep 10s && date".to_string()],
            nonempty!["uname".to_string()],
            "ubuntu:20.04".into(),
            Some(vec!["test".into()])
        ),
    ];
    let pl = Pipeline::new(steps);
    let mut b = Build::new(pl, BuildState::BuildReady, vec![] as CompletedSteps);
    loop {
        b.progress(&conn).await;
        match b.state {
            BuildState::BuildFinished(_res) => {
                break;
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        }
    }
    println!("{:?}", b.completed_steps);
}
// #[cfg(test)]
// mod tests {
//     use std::collections::HashMap;

//     use super::core::{build::*, *};
//     use nonempty::nonempty;
//     // #[test]
//     fn test_pipeline() -> Pipeline {
//         let steps = nonempty![
//             //
//             Step::new(
//                 "First Step".to_string(),
//                 "ubuntu".to_string(),
//                 nonempty!["date".to_string()],
//             ),
//             Step::new(
//                 "Second Step".to_string(),
//                 "ubuntu".to_string(),
//                 nonempty!["uname -r".to_string()],
//             ),
//         ];
//         let pl = Pipeline::new(steps);
//         pl
//     }
//     #[test]
//     fn test_build() {
//         // let mut b = Build::new(
//         //     test_pipeline(),
//         //     BuildState::BuildReady,
//         //     HashMap::new() as CompletedSteps,
//         // );
//         // b.progress();
//         // // println!("{:?}", b.state);
//         // assert_eq!(
//         //     b.state,
//         //     BuildState::BuildRunning(BuildRunningState {
//         //         step: StepName("First Step".to_string())
//         //     })
//         // );
//         // b.progress();
//         // assert_eq!(b.state, BuildState::BuildReady)
//     }
// }
