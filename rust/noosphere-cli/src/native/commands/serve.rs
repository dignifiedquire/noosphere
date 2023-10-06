//! Concrete implementations of subcommands related to running a Noosphere
//! gateway server

use anyhow::Result;

use std::net::{IpAddr, TcpListener};

use url::Url;

use crate::native::workspace::Workspace;

use noosphere_gateway::{start_gateway, DocTicket, GatewayScope};

/// Start a Noosphere gateway server
pub async fn serve(
    interface: IpAddr,
    port: u16,
    ipfs_api: Url,
    iroh_ticket: DocTicket,
    name_resolver_api: Url,
    cors_origin: Option<Url>,
    workspace: &Workspace,
) -> Result<()> {
    workspace.ensure_sphere_initialized()?;

    let listener = TcpListener::bind((interface, port))?;

    let counterpart = workspace.counterpart_identity().await?;

    let identity = workspace.sphere_identity().await?;

    let gateway_scope = GatewayScope {
        identity,
        counterpart,
    };

    let sphere_context = workspace.sphere_context().await?;

    start_gateway(
        listener,
        gateway_scope,
        sphere_context,
        ipfs_api,
        iroh_ticket,
        name_resolver_api,
        cors_origin,
    )
    .await
}
