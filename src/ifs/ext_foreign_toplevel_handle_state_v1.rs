use {
    crate::{
        client::{Client, ClientError},
        leaks::Tracker,
        object::{Object, Version},
        wire::{ExtForeignToplevelHandleStateV1Id, ext_foreign_toplevel_handle_state_v1::*},
    },
    std::rc::Rc,
    thiserror::Error,
};

const STATE_ACTIVATED: u32 = 4;
const STATE_FULLSCREEN: u32 = 8;

pub struct ExtForeignToplevelHandleStateV1 {
    pub id: ExtForeignToplevelHandleStateV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub version: Version,
}

impl ExtForeignToplevelHandleStateV1 {
    pub fn send_state(&self, active: bool, fullscreen: bool) {
        self.client.event(State {
            self_id: self.id,
            states: if active { STATE_ACTIVATED } else { 0 }
                | if fullscreen { STATE_FULLSCREEN } else { 0 },
        });
    }
}

object_base! {
    self = ExtForeignToplevelHandleStateV1;
    version = self.version;
}

impl ExtForeignToplevelHandleStateV1RequestHandler for ExtForeignToplevelHandleStateV1 {
    type Error = ExtForeignToplevelHandleStateV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.client.remove_obj(self)?;
        Ok(())
    }
}

impl Object for ExtForeignToplevelHandleStateV1 {}

simple_add_obj!(ExtForeignToplevelHandleStateV1);

#[derive(Debug, Error)]
pub enum ExtForeignToplevelHandleStateV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(ExtForeignToplevelHandleStateV1Error, ClientError);
