use {
    super::ext_foreign_toplevel_handle_state_v1::ExtForeignToplevelHandleStateV1,
    crate::{
        client::{Client, ClientError},
        leaks::Tracker,
        object::{Object, Version},
        tree::ToplevelOpt,
        utils::clonecell::CloneCell,
        wire::{ExtForeignToplevelHandleV1Id, ext_foreign_toplevel_handle_v1::*},
    },
    std::rc::Rc,
    thiserror::Error,
};

pub struct ExtForeignToplevelHandleV1 {
    pub id: ExtForeignToplevelHandleV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub toplevel: ToplevelOpt,
    pub version: Version,
    pub toplevel_state: CloneCell<Option<Rc<ExtForeignToplevelHandleStateV1>>>,
}

impl ExtForeignToplevelHandleV1 {
    fn detach(&self) {
        if let Some(tl) = self.toplevel.get() {
            tl.tl_data().handles.remove(&(self.client.id, self.id));
        }
    }
}

impl ExtForeignToplevelHandleV1RequestHandler for ExtForeignToplevelHandleV1 {
    type Error = ExtForeignToplevelHandleV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.detach();
        self.client.remove_obj(self)?;
        Ok(())
    }
}

impl ExtForeignToplevelHandleV1 {
    pub fn send_closed(&self) {
        self.client.event(Closed { self_id: self.id });
    }

    pub fn send_done(&self) {
        self.client.event(Done { self_id: self.id });
    }

    pub fn send_title(&self, title: &str) {
        self.client.event(Title {
            self_id: self.id,
            title,
        });
    }

    pub fn send_app_id(&self, app_id: &str) {
        self.client.event(AppId {
            self_id: self.id,
            app_id,
        });
    }

    pub fn send_identifier(&self, identifier: &str) {
        self.client.event(Identifier {
            self_id: self.id,
            identifier,
        });
    }

    pub fn send_state(&self, active: bool, fullscreen: bool) {
        if let Some(state) = self.toplevel_state.get() {
            state.send_state(active, fullscreen);
        }
    }
}

object_base! {
    self = ExtForeignToplevelHandleV1;
    version = self.version;
}

impl Object for ExtForeignToplevelHandleV1 {
    fn break_loops(&self) {
        self.detach();
    }
}

dedicated_add_obj!(
    ExtForeignToplevelHandleV1,
    ExtForeignToplevelHandleV1Id,
    foreign_toplevel_handles
);

#[derive(Debug, Error)]
pub enum ExtForeignToplevelHandleV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(ExtForeignToplevelHandleV1Error, ClientError);
