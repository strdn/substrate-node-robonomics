///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2019 Airalab <research@aira.life> 
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! Console line interface.

use log::info;
use std::ops::Deref;
use std::cell::RefCell;
use futures::future;
use futures::sync::oneshot;
use tokio::prelude::Future;
use tokio::runtime::Runtime;
use substrate_cli::{informant, parse_and_execute, NoCustom};
use substrate_service::{ServiceFactory, Roles as ServiceRoles};
pub use substrate_cli::{VersionInfo, IntoExit, error};

mod chain_spec;
mod service;

/// Parse command line arguments into service configuration.
pub fn run<I, T, E>(args: I, exit: E, version: VersionInfo) -> error::Result<()> where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
    E: IntoExit,
{
    parse_and_execute::<service::Factory, NoCustom, NoCustom, _, _, _, _, _>(
        load_spec, &version, "robonomics-node", args, exit,
         |exit, _cli_args, _custom_args, config| {
            info!("{}", version.name);
            info!("  version {}", config.full_version());
            info!("  by {}, 2018, 2019", version.author);
            info!("Chain specification: {}", config.chain_spec.name());
            info!("Node name: {}", config.name);
            info!("Roles: {:?}", config.roles);
            let runtime = Runtime::new().map_err(|e| format!("{:?}", e))?;
            match config.roles {
                ServiceRoles::LIGHT => run_until_exit(
                    runtime,
                    service::Factory::new_light(config).map_err(|e| format!("{:?}", e))?,
                    exit
                ),
                _ => run_until_exit(
                    runtime,
                    service::Factory::new_full(config).map_err(|e| format!("{:?}", e))?,
                    exit
                ),
            }.map_err(|e| format!("{:?}", e))
        }
    ).map_err(Into::into).map(|_| ())
}

fn load_spec(id: &str) -> Result<Option<chain_spec::ChainSpec>, String> {
    Ok(match chain_spec::ChainOpt::from(id) {
        Some(spec) => Some(spec.load()?),
        None => None,
    })
}

fn run_until_exit<T, C, E>(
    mut runtime: Runtime,
    service: T,
    e: E,
) -> error::Result<()>
    where
        T: Deref<Target=substrate_service::Service<C>> + Future<Item=(), Error=()> + Send + 'static,
        C: substrate_service::Components,
        E: IntoExit,
{
    let (exit_send, exit) = exit_future::signal();

    let informant = informant::build(&service);
    runtime.executor().spawn(exit.until(informant).map(|_| ()));

    let _telemetry = service.telemetry();

    let _ = runtime.block_on(service.select(e.into_exit()));
    exit_send.fire();

    Ok(())
}

// handles ctrl-c
pub struct Exit;
impl IntoExit for Exit {
    type Exit = future::MapErr<oneshot::Receiver<()>, fn(oneshot::Canceled) -> ()>;
    fn into_exit(self) -> Self::Exit {
        // can't use signal directly here because CtrlC takes only `Fn`.
        let (exit_send, exit) = oneshot::channel();

        let exit_send_cell = RefCell::new(Some(exit_send));
        ctrlc::set_handler(move || {
            if let Some(exit_send) = exit_send_cell.try_borrow_mut().expect("signal handler not reentrant; qed").take() {
                exit_send.send(()).expect("Error sending exit notification");
            }
        }).expect("Error setting Ctrl-C handler");

        exit.map_err(drop)
    }
}
