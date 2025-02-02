use {
    super::ext_foreign_toplevel_handle_state_v1::ExtForeignToplevelHandleStateV1,
    crate::{
        client::{CAP_FOREIGN_TOPLEVEL_STATE, Client, ClientCaps, ClientError},
        globals::{Global, GlobalName},
        leaks::Tracker,
        object::{Object, Version},
        wire::{ExtForeignToplevelStateV1Id, ext_foreign_toplevel_state_v1::*},
    },
    std::rc::Rc,
    thiserror::Error,
};

const STATE_ACTIVATED: u32 = 4;
const STATE_FULLSCREEN: u32 = 8;

pub struct ExtForeignToplevelStateV1Global {
    pub name: GlobalName,
}

impl ExtForeignToplevelStateV1Global {
    pub fn new(name: GlobalName) -> Self {
        Self { name }
    }

    fn bind_(
        self: Rc<Self>,
        id: ExtForeignToplevelStateV1Id,
        client: &Rc<Client>,
        version: Version,
    ) -> Result<(), ExtForeignToplevelStateV1Error> {
        let obj = Rc::new(ExtForeignToplevelStateV1 {
            id,
            client: client.clone(),
            tracker: Default::default(),
            version,
        });
        track!(client, obj);
        client.add_client_obj(&obj)?;
        client.event(Capabilities {
            self_id: id,
            capabilities: STATE_ACTIVATED | STATE_FULLSCREEN,
        });
        Ok(())
    }
}

pub struct ExtForeignToplevelStateV1 {
    pub id: ExtForeignToplevelStateV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub version: Version,
}

impl ExtForeignToplevelStateV1RequestHandler for ExtForeignToplevelStateV1 {
    type Error = ExtForeignToplevelStateV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn get_handle_state(&self, req: GetHandleState, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        let handle = self.client.lookup(req.handle)?;
        let handle_state = Rc::new(ExtForeignToplevelHandleStateV1 {
            id: req.id,
            client: self.client.clone(),
            tracker: Default::default(),
            version: self.version,
        });
        track!(self.client, handle_state);
        self.client.add_client_obj(&handle_state)?;
        handle.toplevel_state.set(Some(handle_state));
        if let Some(tl) = handle.toplevel.get() {
            tl.tl_data().send_extra_toplevel_state(&handle);
        }
        Ok(())
    }
}

global_base!(
    ExtForeignToplevelStateV1Global,
    ExtForeignToplevelStateV1,
    ExtForeignToplevelStateV1Error
);

impl Global for ExtForeignToplevelStateV1Global {
    fn singleton(&self) -> bool {
        true
    }

    fn version(&self) -> u32 {
        1
    }

    fn required_caps(&self) -> ClientCaps {
        CAP_FOREIGN_TOPLEVEL_STATE
    }
}

simple_add_global!(ExtForeignToplevelStateV1Global);

object_base! {
    self = ExtForeignToplevelStateV1;
    version = self.version;
}

impl Object for ExtForeignToplevelStateV1 {}

simple_add_obj!(ExtForeignToplevelStateV1);

#[derive(Debug, Error)]
pub enum ExtForeignToplevelStateV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(ExtForeignToplevelStateV1Error, ClientError);
