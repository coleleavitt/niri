use futures_util::StreamExt;
use zbus::fdo;
use zbus::names::InterfaceName;

pub enum Login1ToNiri {
    LidClosedChanged(bool),
    PrepareForSleep(bool),
}

pub fn start(
    to_niri: calloop::channel::Sender<Login1ToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::system()?;

    let async_conn = conn.inner().clone();
    let to_niri_lid = to_niri.clone();
    let lid_future = async move {
        let proxy = fdo::PropertiesProxy::new(
            &async_conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
        )
        .await;
        let proxy = match proxy {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating PropertiesProxy: {err:?}");
                return;
            }
        };

        let mut props_changed = match proxy.receive_properties_changed().await {
            Ok(x) => x,
            Err(err) => {
                warn!("error subscribing to PropertiesChanged: {err:?}");
                return;
            }
        };

        let props = proxy
            .get_all(InterfaceName::try_from("org.freedesktop.login1.Manager").unwrap())
            .await;
        let mut props = match props {
            Ok(x) => x,
            Err(err) => {
                warn!("error receiving initial properties: {err:?}");
                return;
            }
        };

        trace!("initial properties: {props:?}");

        let mut lid_closed = props
            .remove("LidClosed")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or_default();

        if let Err(err) = to_niri_lid.send(Login1ToNiri::LidClosedChanged(lid_closed)) {
            warn!("error sending initial lid state to niri: {err:?}");
            return;
        };

        while let Some(signal) = props_changed.next().await {
            let args = match signal.args() {
                Ok(args) => args,
                Err(err) => {
                    warn!("error parsing PropertiesChanged args: {err:?}");
                    return;
                }
            };

            let mut new_lid_closed = lid_closed;
            let mut changed = false;
            for (name, value) in args.changed_properties() {
                trace!("changed property: {name} => {value:?}");
                if *name != "LidClosed" {
                    continue;
                }

                new_lid_closed = bool::try_from(value).unwrap_or(new_lid_closed);
                changed = true;
            }

            if !changed {
                continue;
            }

            if new_lid_closed == lid_closed {
                continue;
            }

            lid_closed = new_lid_closed;
            if let Err(err) = to_niri_lid.send(Login1ToNiri::LidClosedChanged(lid_closed)) {
                warn!("error sending message to niri: {err:?}");
                return;
            };
        }
    };

    let async_conn = conn.inner().clone();
    let sleep_future = async move {
        let mut inhibitor = acquire_sleep_inhibitor(&async_conn).await;

        let rule = match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.login1")
            .and_then(|b| b.path("/org/freedesktop/login1"))
            .and_then(|b| b.interface("org.freedesktop.login1.Manager"))
            .and_then(|b| b.member("PrepareForSleep"))
        {
            Ok(b) => b.build(),
            Err(err) => {
                warn!("error building PrepareForSleep match rule: {err:?}");
                return;
            }
        };

        let mut stream = match zbus::MessageStream::for_match_rule(rule, &async_conn, Some(1)).await
        {
            Ok(s) => s,
            Err(err) => {
                warn!("error subscribing to PrepareForSleep: {err:?}");
                return;
            }
        };

        while let Some(msg) = stream.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(err) => {
                    warn!("error receiving PrepareForSleep signal: {err:?}");
                    continue;
                }
            };

            let going_to_sleep: bool = match msg.body().deserialize() {
                Ok(v) => v,
                Err(err) => {
                    warn!("error parsing PrepareForSleep body: {err:?}");
                    continue;
                }
            };

            debug!("PrepareForSleep: going_to_sleep={going_to_sleep}");

            if let Err(err) = to_niri.send(Login1ToNiri::PrepareForSleep(going_to_sleep)) {
                warn!("error sending PrepareForSleep to niri: {err:?}");
                return;
            }

            if going_to_sleep {
                // Release inhibitor to allow sleep to proceed.
                inhibitor = None;
            } else {
                // Re-acquire inhibitor for next sleep cycle.
                inhibitor = acquire_sleep_inhibitor(&async_conn).await;
            }
        }

        drop(inhibitor);
    };

    let task = conn
        .inner()
        .executor()
        .spawn(lid_future, "monitor login1 property changes");
    task.detach();

    let task = conn
        .inner()
        .executor()
        .spawn(sleep_future, "monitor PrepareForSleep");
    task.detach();

    Ok(conn)
}

async fn acquire_sleep_inhibitor(conn: &zbus::Connection) -> Option<zbus::zvariant::OwnedFd> {
    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &("sleep", "niri", "Preparing display for sleep", "delay"),
        )
        .await;

    match reply {
        Ok(msg) => match msg.body().deserialize::<zbus::zvariant::OwnedFd>() {
            Ok(fd) => {
                debug!("acquired sleep inhibitor lock");
                Some(fd)
            }
            Err(err) => {
                warn!("error parsing Inhibit response: {err:?}");
                None
            }
        },
        Err(err) => {
            warn!("error acquiring sleep inhibitor: {err:?}");
            None
        }
    }
}
