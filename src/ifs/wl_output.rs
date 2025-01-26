mod removed_output;

use {
    crate::{
        backend,
        client::{Client, ClientError, ClientId},
        format::{Format, XRGB8888},
        globals::{Global, GlobalName},
        ifs::{wl_surface::WlSurface, zxdg_output_v1::ZxdgOutputV1},
        leaks::Tracker,
        object::{Object, Version},
        rect::Rect,
        state::{ConnectorData, State},
        tree::{calculate_logical_size, OutputNode, TearingMode, VrrMode},
        utils::{
            cell_ext::CellExt, clonecell::CloneCell, copyhashmap::CopyHashMap, rc_eq::rc_eq,
            transform_ext::TransformExt,
        },
        wire::{wl_output::*, WlOutputId, ZxdgOutputV1Id},
    },
    ahash::AHashMap,
    jay_config::video::Transform,
    std::{
        cell::{Cell, RefCell},
        collections::hash_map::Entry,
        rc::Rc,
    },
    thiserror::Error,
};

const SP_UNKNOWN: i32 = 0;
#[expect(dead_code)]
const SP_NONE: i32 = 1;
#[expect(dead_code)]
const SP_HORIZONTAL_RGB: i32 = 2;
#[expect(dead_code)]
const SP_HORIZONTAL_BGR: i32 = 3;
#[expect(dead_code)]
const SP_VERTICAL_RGB: i32 = 4;
#[expect(dead_code)]
const SP_VERTICAL_BGR: i32 = 5;

pub const TF_NORMAL: i32 = 0;
pub const TF_90: i32 = 1;
pub const TF_180: i32 = 2;
pub const TF_270: i32 = 3;
pub const TF_FLIPPED: i32 = 4;
pub const TF_FLIPPED_90: i32 = 5;
pub const TF_FLIPPED_180: i32 = 6;
pub const TF_FLIPPED_270: i32 = 7;

const MODE_CURRENT: u32 = 1;
#[expect(dead_code)]
const MODE_PREFERRED: u32 = 2;

pub struct WlOutputGlobal {
    pub name: GlobalName,
    pub _state: Rc<State>,
    pub connector: Rc<ConnectorData>,
    pub pos: Cell<Rect>,
    pub output_id: Rc<OutputId>,
    pub mode: Cell<backend::Mode>,
    pub refresh_nsec: Cell<u64>,
    pub modes: Vec<backend::Mode>,
    pub formats: CloneCell<Rc<Vec<&'static Format>>>,
    pub format: Cell<&'static Format>,
    pub width_mm: i32,
    pub height_mm: i32,
    pub bindings: RefCell<AHashMap<ClientId, AHashMap<WlOutputId, Rc<WlOutput>>>>,
    pub destroyed: Cell<bool>,
    pub legacy_scale: Cell<u32>,
    pub persistent: Rc<PersistentOutputState>,
    pub opt: Rc<OutputGlobalOpt>,
}

#[derive(Default)]
pub struct OutputGlobalOpt {
    pub global: CloneCell<Option<Rc<WlOutputGlobal>>>,
    pub node: CloneCell<Option<Rc<OutputNode>>>,
}

impl OutputGlobalOpt {
    pub fn get(&self) -> Option<Rc<WlOutputGlobal>> {
        self.global.get()
    }

    pub fn node(&self) -> Option<Rc<OutputNode>> {
        self.node.get()
    }

    pub fn clear(&self) {
        self.node.take();
        self.global.take();
    }
}

pub struct PersistentOutputState {
    pub transform: Cell<Transform>,
    pub scale: Cell<crate::scale::Scale>,
    pub pos: Cell<(i32, i32)>,
    pub vrr_mode: Cell<&'static VrrMode>,
    pub vrr_cursor_hz: Cell<Option<f64>>,
    pub tearing_mode: Cell<&'static TearingMode>,
}

#[derive(Eq, PartialEq, Hash, Debug)]
pub struct OutputId {
    pub connector: Option<String>,
    pub manufacturer: String,
    pub model: String,
    pub serial_number: String,
}

impl OutputId {
    pub fn new(
        connector: String,
        manufacturer: String,
        model: String,
        serial_number: String,
    ) -> Self {
        Self {
            connector: serial_number.is_empty().then_some(connector),
            manufacturer,
            model,
            serial_number,
        }
    }
}

impl WlOutputGlobal {
    pub fn clear(&self) {
        self.opt.clear();
        self.bindings.borrow_mut().clear();
    }

    pub fn new(
        name: GlobalName,
        state: &Rc<State>,
        connector: &Rc<ConnectorData>,
        modes: Vec<backend::Mode>,
        mode: &backend::Mode,
        width_mm: i32,
        height_mm: i32,
        output_id: &Rc<OutputId>,
        persistent_state: &Rc<PersistentOutputState>,
    ) -> Self {
        let (x, y) = persistent_state.pos.get();
        let scale = persistent_state.scale.get();
        let (width, height) = calculate_logical_size(
            (mode.width, mode.height),
            persistent_state.transform.get(),
            scale,
        );
        Self {
            name,
            _state: state.clone(),
            connector: connector.clone(),
            pos: Cell::new(Rect::new_sized(x, y, width, height).unwrap()),
            output_id: output_id.clone(),
            mode: Cell::new(*mode),
            refresh_nsec: Cell::new(mode.refresh_nsec()),
            modes,
            formats: CloneCell::new(Rc::new(vec![])),
            format: Cell::new(XRGB8888),
            width_mm,
            height_mm,
            bindings: Default::default(),
            destroyed: Cell::new(false),
            legacy_scale: Cell::new(scale.round_up()),
            persistent: persistent_state.clone(),
            opt: Default::default(),
        }
    }

    pub fn position(&self) -> Rect {
        self.pos.get()
    }

    pub fn for_each_binding<F: FnMut(&Rc<WlOutput>)>(&self, client: ClientId, mut f: F) {
        let bindings = self.bindings.borrow_mut();
        if let Some(bindings) = bindings.get(&client) {
            for binding in bindings.values() {
                f(binding);
            }
        }
    }

    pub fn send_enter(&self, surface: &WlSurface) {
        self.for_each_binding(surface.client.id, |b| {
            surface.send_enter(b.id);
        })
    }

    pub fn send_leave(&self, surface: &WlSurface) {
        self.for_each_binding(surface.client.id, |b| {
            surface.send_leave(b.id);
        })
    }

    pub fn send_mode(&self) {
        let bindings = self.bindings.borrow_mut();
        for binding in bindings.values() {
            for binding in binding.values() {
                binding.send_updates();
            }
        }
    }

    fn bind_(
        self: Rc<Self>,
        id: WlOutputId,
        client: &Rc<Client>,
        version: Version,
    ) -> Result<(), WlOutputError> {
        let obj = Rc::new(WlOutput {
            global: self.opt.clone(),
            id,
            xdg_outputs: Default::default(),
            client: client.clone(),
            version,
            tracker: Default::default(),
        });
        track!(client, obj);
        client.add_client_obj(&obj)?;
        self.bindings
            .borrow_mut()
            .entry(client.id)
            .or_default()
            .insert(id, obj.clone());
        obj.send_geometry();
        obj.send_mode();
        if obj.version >= SEND_SCALE_SINCE {
            obj.send_scale();
        }
        if obj.version >= SEND_NAME_SINCE {
            obj.send_name();
        }
        if obj.version >= SEND_DONE_SINCE {
            obj.send_done();
        }
        for group in client.objects.ext_workspace_groups.lock().values() {
            if rc_eq(&group.output, &self.opt) {
                group.handle_new_output(&obj);
            }
        }
        Ok(())
    }

    pub fn pixel_size(&self) -> (i32, i32) {
        let mode = self.mode.get();
        self.persistent
            .transform
            .get()
            .maybe_swap((mode.width, mode.height))
    }
}

global_base!(WlOutputGlobal, WlOutput, WlOutputError);

const OUTPUT_VERSION: u32 = 4;

impl Global for WlOutputGlobal {
    fn singleton(&self) -> bool {
        false
    }

    fn version(&self) -> u32 {
        OUTPUT_VERSION
    }
}

dedicated_add_global!(WlOutputGlobal, outputs);

pub struct WlOutput {
    pub global: Rc<OutputGlobalOpt>,
    pub id: WlOutputId,
    pub xdg_outputs: CopyHashMap<ZxdgOutputV1Id, Rc<ZxdgOutputV1>>,
    client: Rc<Client>,
    pub version: Version,
    tracker: Tracker<Self>,
}

pub const SEND_DONE_SINCE: Version = Version(2);
pub const SEND_SCALE_SINCE: Version = Version(2);
pub const SEND_NAME_SINCE: Version = Version(4);

impl WlOutput {
    pub fn send_updates(&self) {
        self.send_geometry();
        self.send_mode();
        if self.version >= SEND_SCALE_SINCE {
            self.send_scale();
        }
        if self.version >= SEND_DONE_SINCE {
            self.send_done();
        }
        let xdg = self.xdg_outputs.lock();
        for xdg in xdg.values() {
            xdg.send_updates();
        }
    }

    fn send_geometry(&self) {
        let Some(global) = self.global.get() else {
            return;
        };
        let pos = global.pos.get();
        let mut x = pos.x1();
        let mut y = pos.y1();
        logical_to_client_wire_scale!(self.client, x, y);
        let event = Geometry {
            self_id: self.id,
            x,
            y,
            physical_width: global.width_mm,
            physical_height: global.height_mm,
            subpixel: SP_UNKNOWN,
            make: &global.output_id.manufacturer,
            model: &global.output_id.model,
            transform: global.persistent.transform.get().to_wl(),
        };
        self.client.event(event);
    }

    fn send_mode(&self) {
        let Some(global) = self.global.get() else {
            return;
        };
        let mut mode = global.mode.get();
        logical_to_client_wire_scale!(self.client, mode.width, mode.height);
        let event = Mode {
            self_id: self.id,
            flags: MODE_CURRENT,
            width: mode.width,
            height: mode.height,
            refresh: mode.refresh_rate_millihz as _,
        };
        self.client.event(event);
    }

    fn send_scale(&self) {
        let Some(global) = self.global.get() else {
            return;
        };
        let factor = match self.client.wire_scale.is_some() {
            true => 1,
            false => global.legacy_scale.get() as _,
        };
        let event = Scale {
            self_id: self.id,
            factor,
        };
        self.client.event(event);
    }

    fn send_name(&self) {
        let Some(global) = self.global.get() else {
            return;
        };
        self.client.event(Name {
            self_id: self.id,
            name: &global.connector.name,
        });
    }

    pub fn send_done(&self) {
        let event = Done { self_id: self.id };
        self.client.event(event);
    }

    fn remove_binding(&self) {
        let Some(global) = self.global.get() else {
            return;
        };
        if let Entry::Occupied(mut e) = global.bindings.borrow_mut().entry(self.client.id) {
            e.get_mut().remove(&self.id);
            if e.get().is_empty() {
                e.remove();
            }
        };
    }
}

impl WlOutputRequestHandler for WlOutput {
    type Error = WlOutputError;

    fn release(&self, _req: Release, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.xdg_outputs.clear();
        self.remove_binding();
        self.client.remove_obj(self)?;
        Ok(())
    }
}

object_base! {
    self = WlOutput;
    version = self.version;
}

impl Object for WlOutput {
    fn break_loops(&self) {
        self.xdg_outputs.clear();
        self.remove_binding();
    }
}

dedicated_add_obj!(WlOutput, WlOutputId, outputs);

#[derive(Debug, Error)]
pub enum WlOutputError {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(WlOutputError, ClientError);
