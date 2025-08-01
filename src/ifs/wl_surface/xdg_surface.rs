pub mod xdg_popup;
pub mod xdg_toplevel;

use {
    crate::{
        client::ClientError,
        ifs::{
            wl_surface::{
                PendingState, SurfaceExt, SurfaceRole, WlSurface, WlSurfaceError,
                tray::TrayItemId,
                xdg_surface::{
                    xdg_popup::{XdgPopup, XdgPopupError, XdgPopupParent},
                    xdg_toplevel::{WM_CAPABILITIES_SINCE, XdgToplevel},
                },
            },
            xdg_wm_base::XdgWmBase,
        },
        leaks::Tracker,
        object::Object,
        rect::Rect,
        tree::{
            FindTreeResult, FoundNode, Node, NodeLayerLink, NodeLocation, OutputNode, StackedNode,
            WorkspaceNode,
        },
        utils::{
            clonecell::CloneCell,
            copyhashmap::CopyHashMap,
            hash_map_ext::HashMapExt,
            linkedlist::{LinkedList, LinkedNode},
            numcell::NumCell,
            option_ext::OptionExt,
            rc_eq::rc_eq,
        },
        wire::{WlSurfaceId, XdgPopupId, XdgSurfaceId, xdg_surface::*},
    },
    std::{
        cell::{Cell, RefCell, RefMut},
        fmt::Debug,
        rc::Rc,
    },
    thiserror::Error,
};

#[expect(dead_code)]
const NOT_CONSTRUCTED: u32 = 1;
const ALREADY_CONSTRUCTED: u32 = 2;
#[expect(dead_code)]
const UNCONFIGURED_BUFFER: u32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum XdgSurfaceRole {
    None,
    XdgPopup,
    XdgToplevel,
}

impl XdgSurfaceRole {
    fn name(self) -> &'static str {
        match self {
            XdgSurfaceRole::None => "none",
            XdgSurfaceRole::XdgPopup => "xdg_popup",
            XdgSurfaceRole::XdgToplevel => "xdg_toplevel",
        }
    }
}

pub struct XdgSurface {
    id: XdgSurfaceId,
    base: Rc<XdgWmBase>,
    role: Cell<XdgSurfaceRole>,
    pub surface: Rc<WlSurface>,
    requested_serial: NumCell<u32>,
    acked_serial: Cell<Option<u32>>,
    geometry: Cell<Option<Rect>>,
    extents: Cell<Rect>,
    pub absolute_desired_extents: Cell<Rect>,
    ext: CloneCell<Option<Rc<dyn XdgSurfaceExt>>>,
    popup_display_stack: CloneCell<Rc<LinkedList<Rc<dyn StackedNode>>>>,
    is_above_layers: Cell<bool>,
    popups: CopyHashMap<XdgPopupId, Rc<Popup>>,
    pub workspace: CloneCell<Option<Rc<WorkspaceNode>>>,
    pub tracker: Tracker<Self>,
    have_initial_commit: Cell<bool>,
}

struct Popup {
    parent: Rc<XdgSurface>,
    popup: Rc<XdgPopup>,
    display_link: RefCell<Option<LinkedNode<Rc<dyn StackedNode>>>>,
    workspace_link: RefCell<Option<LinkedNode<Rc<dyn StackedNode>>>>,
}

impl XdgPopupParent for Popup {
    fn position(&self) -> Rect {
        self.parent.absolute_desired_extents.get()
    }

    fn remove_popup(&self) {
        self.parent.popups.remove(&self.popup.id);
    }

    fn output(&self) -> Rc<OutputNode> {
        self.parent.surface.output.get()
    }

    fn has_workspace_link(&self) -> bool {
        self.workspace_link.borrow().is_some()
    }

    fn post_commit(&self) {
        let mut wl = self.workspace_link.borrow_mut();
        let mut dl = self.display_link.borrow_mut();
        let surface = &self.popup.xdg.surface;
        let state = &surface.client.state;
        if surface.buffer.is_some() {
            let mut any_set = false;
            if wl.is_none()
                && let Some(ws) = self.parent.workspace.get()
            {
                self.popup.xdg.set_workspace(&ws);
                *wl = Some(ws.stacked.add_last(self.popup.clone()));
                any_set = true;
            }
            if dl.is_none() {
                *dl = Some(
                    self.parent
                        .popup_display_stack
                        .get()
                        .add_last(self.popup.clone()),
                );
                any_set = true;
            }
            if any_set {
                state.tree_changed();
                self.popup.set_visible(self.parent.surface.visible.get());
            }
        } else {
            if wl.take().is_some() {
                drop(wl);
                drop(dl);
                self.popup.set_visible(false);
                self.popup.destroy_node();
            }
        }
    }

    fn visible(&self) -> bool {
        self.parent.surface.visible.get()
    }

    fn make_visible(self: Rc<Self>) {
        if let Some(ext) = self.parent.ext.get() {
            ext.make_visible();
        }
    }

    fn node_layer(&self) -> NodeLayerLink {
        let Some(link) = self.display_link.borrow().as_ref().map(|w| w.to_ref()) else {
            return NodeLayerLink::Display;
        };
        match self.popup.xdg.is_above_layers.get() {
            true => NodeLayerLink::StackedAboveLayers(link),
            false => NodeLayerLink::Stacked(link),
        }
    }

    fn tray_item(&self) -> Option<TrayItemId> {
        self.parent.clone().tray_item()
    }
}

#[derive(Default, Debug)]
pub struct PendingXdgSurfaceData {
    geometry: Option<Rect>,
}

impl PendingXdgSurfaceData {
    pub fn merge(&mut self, next: &mut Self) {
        macro_rules! opt {
            ($name:ident) => {
                if let Some(n) = next.$name.take() {
                    self.$name = Some(n);
                }
            };
        }
        opt!(geometry);
    }
}

pub trait XdgSurfaceExt: Debug {
    fn initial_configure(self: Rc<Self>) -> Result<(), XdgSurfaceError> {
        Ok(())
    }

    fn post_commit(self: Rc<Self>) {
        // nothing
    }

    fn extents_changed(&self) {
        // nothing
    }

    fn geometry_changed(&self) {
        // nothing
    }

    fn focus_node(&self) -> Option<Rc<dyn Node>> {
        None
    }

    fn tray_item(&self) -> Option<TrayItemId> {
        None
    }

    fn make_visible(self: Rc<Self>);

    fn node_layer(&self) -> NodeLayerLink;
}

impl XdgSurface {
    pub fn new(wm_base: &Rc<XdgWmBase>, id: XdgSurfaceId, surface: &Rc<WlSurface>) -> Self {
        Self {
            id,
            base: wm_base.clone(),
            role: Cell::new(XdgSurfaceRole::None),
            surface: surface.clone(),
            requested_serial: NumCell::new(1),
            acked_serial: Cell::new(None),
            geometry: Cell::new(None),
            extents: Cell::new(surface.extents.get()),
            absolute_desired_extents: Cell::new(Default::default()),
            ext: Default::default(),
            popup_display_stack: CloneCell::new(surface.client.state.root.stacked.clone()),
            is_above_layers: Cell::new(false),
            popups: Default::default(),
            workspace: Default::default(),
            tracker: Default::default(),
            have_initial_commit: Default::default(),
        }
    }

    fn update_surface_position(&self) {
        let (mut x1, mut y1) = self.absolute_desired_extents.get().position();
        if let Some(geo) = self.geometry.get() {
            x1 -= geo.x1();
            y1 -= geo.y1();
        }
        self.surface.set_absolute_position(x1, y1);
        self.update_popup_positions();
    }

    fn set_absolute_desired_extents(&self, ext: &Rect) {
        let prev = self.absolute_desired_extents.replace(*ext);
        if ext.position() != prev.position() {
            self.update_surface_position();
        }
    }

    fn set_workspace(&self, ws: &Rc<WorkspaceNode>) {
        self.workspace.set(Some(ws.clone()));
        self.surface.set_output(&ws.output.get(), ws.location());
        let pu = self.popups.lock();
        for pu in pu.values() {
            pu.popup.xdg.set_workspace(ws);
        }
    }

    pub fn set_output(&self, output: &Rc<OutputNode>) {
        self.surface
            .set_output(output, NodeLocation::Output(output.id));
        let pu = self.popups.lock();
        for pu in pu.values() {
            pu.popup.xdg.set_output(output);
        }
    }

    fn set_role(&self, role: XdgSurfaceRole) -> Result<(), XdgSurfaceError> {
        use XdgSurfaceRole::*;
        match (self.role.get(), role) {
            (None, _) => {}
            (old, new) if old == new => {}
            (old, new) => {
                return Err(XdgSurfaceError::IncompatibleRole {
                    id: self.id,
                    old,
                    new,
                });
            }
        }
        self.role.set(role);
        Ok(())
    }

    fn destroy_node(&self) {
        self.workspace.set(None);
        self.surface.destroy_node();
        for popup in self.popups.lock().drain_values() {
            popup.popup.destroy_node();
        }
    }

    fn detach_node(&self) {
        self.workspace.set(None);
        self.surface.detach_node(false);
        let popups = self.popups.lock();
        for popup in popups.values() {
            let _v = popup.workspace_link.borrow_mut().take();
            popup.popup.detach_node();
        }
    }

    pub fn damage(&self) {
        let (x, y) = self.surface.buffer_abs_pos.get().position();
        let extents = self.surface.extents.get();
        self.surface.client.state.damage(extents.move_(x, y));
    }

    pub fn geometry(&self) -> Option<Rect> {
        self.geometry.get()
    }

    pub fn do_send_configure(&self) {
        let serial = self.requested_serial.fetch_add(1) + 1;
        self.send_configure(serial);
    }

    pub fn send_configure(&self, serial: u32) {
        self.surface.client.event(Configure {
            self_id: self.id,
            serial,
        })
    }

    pub fn install(self: &Rc<Self>) -> Result<(), XdgSurfaceError> {
        self.surface.set_role(SurfaceRole::XdgSurface)?;
        if self.surface.ext.get().is_some() {
            return Err(XdgSurfaceError::AlreadyAttached(self.surface.id));
        }
        self.surface.ext.set(self.clone());
        Ok(())
    }

    fn pending(&self) -> RefMut<Box<PendingXdgSurfaceData>> {
        RefMut::map(self.surface.pending.borrow_mut(), |p| {
            p.xdg_surface.get_or_insert_default_ext()
        })
    }

    pub fn set_popup_stack(
        &self,
        stack: &Rc<LinkedList<Rc<dyn StackedNode>>>,
        is_above_layers: bool,
    ) {
        self.is_above_layers.set(is_above_layers);
        let prev = self.popup_display_stack.set(stack.clone());
        if rc_eq(&prev, stack) {
            return;
        }
        for popup in self.popups.lock().values() {
            if let Some(dl) = &*popup.display_link.borrow() {
                stack.add_last_existing(dl);
            }
            popup.popup.xdg.set_popup_stack(stack, is_above_layers);
        }
    }

    pub fn for_each_popup(&self, mut f: impl FnMut(&Rc<XdgPopup>)) {
        for popup in self.popups.lock().values() {
            f(&popup.popup);
        }
    }
}

impl XdgSurfaceRequestHandler for XdgSurface {
    type Error = XdgSurfaceError;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        if self.ext.is_some() {
            return Err(XdgSurfaceError::RoleNotYetDestroyed(self.id));
        }
        {
            let children = self.popups.lock();
            if !children.is_empty() {
                return Err(XdgSurfaceError::PopupsNotYetDestroyed);
            }
        }
        self.surface.unset_ext();
        self.base.surfaces.remove(&self.id);
        self.surface.client.remove_obj(self)?;
        Ok(())
    }

    fn get_toplevel(&self, req: GetToplevel, slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.set_role(XdgSurfaceRole::XdgToplevel)?;
        if self.ext.is_some() {
            self.surface.client.protocol_error(
                self,
                ALREADY_CONSTRUCTED,
                &format!(
                    "wl_surface {} already has an assigned xdg_toplevel",
                    self.surface.id
                ),
            );
            return Err(XdgSurfaceError::AlreadyConstructed);
        }
        let toplevel = Rc::new_cyclic(|weak| XdgToplevel::new(req.id, slf, weak));
        track!(self.surface.client, toplevel);
        self.surface.client.add_client_obj(&toplevel)?;
        self.ext.set(Some(toplevel.clone()));
        if self.base.version >= WM_CAPABILITIES_SINCE {
            toplevel.send_wm_capabilities();
        }
        self.surface.set_toplevel(Some(toplevel));
        Ok(())
    }

    fn get_popup(&self, req: GetPopup, slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.set_role(XdgSurfaceRole::XdgPopup)?;
        let mut parent = None;
        if req.parent.is_some() {
            parent = Some(self.surface.client.lookup(req.parent)?);
        }
        let positioner = self.surface.client.lookup(req.positioner)?;
        if self.ext.is_some() {
            self.surface.client.protocol_error(
                self,
                ALREADY_CONSTRUCTED,
                &format!(
                    "wl_surface {} already has an assigned xdg_popup",
                    self.surface.id
                ),
            );
            return Err(XdgSurfaceError::AlreadyConstructed);
        }
        let popup = Rc::new(XdgPopup::new(req.id, slf, &positioner)?);
        track!(self.surface.client, popup);
        self.surface.client.add_client_obj(&popup)?;
        if let Some(parent) = &parent {
            let user = Rc::new(Popup {
                parent: parent.clone(),
                popup: popup.clone(),
                display_link: Default::default(),
                workspace_link: Default::default(),
            });
            popup.parent.set(Some(user.clone()));
            popup.xdg.set_popup_stack(
                &parent.popup_display_stack.get(),
                parent.is_above_layers.get(),
            );
            popup.xdg.set_output(&parent.surface.output.get());
            parent.popups.set(req.id, user);
        }
        self.ext.set(Some(popup));
        Ok(())
    }

    fn set_window_geometry(
        &self,
        req: SetWindowGeometry,
        _slf: &Rc<Self>,
    ) -> Result<(), Self::Error> {
        if req.height == 0 && req.width == 0 {
            // TODO: https://crbug.com/1329214
            return Ok(());
        }
        if req.height <= 0 || req.width <= 0 {
            return Err(XdgSurfaceError::NonPositiveWidthHeight);
        }
        let extents = Rect::new_sized(req.x, req.y, req.width, req.height).unwrap();
        self.pending().geometry = Some(extents);
        Ok(())
    }

    fn ack_configure(&self, req: AckConfigure, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        if self.requested_serial.get() == req.serial {
            self.acked_serial.set(Some(req.serial));
        }
        Ok(())
    }
}

impl XdgSurface {
    fn update_extents(&self) {
        let old_extents = self.extents.get();
        let mut new_extents = self.surface.extents.get();
        if let Some(geometry) = self.geometry.get() {
            new_extents = new_extents.intersect(geometry);
        }
        self.extents.set(new_extents);
        if old_extents != new_extents
            && let Some(ext) = self.ext.get()
        {
            ext.extents_changed();
        }
    }

    fn find_tree_at(&self, mut x: i32, mut y: i32, tree: &mut Vec<FoundNode>) -> FindTreeResult {
        if let Some(geo) = self.geometry.get() {
            let (xt, yt) = geo.translate_inv(x, y);
            x = xt;
            y = yt;
        }
        self.surface.find_tree_at_(x, y, tree)
    }

    fn update_popup_positions(&self) {
        let popups = self.popups.lock();
        for popup in popups.values() {
            popup.popup.update_absolute_position();
        }
    }

    fn set_visible(&self, visible: bool) {
        self.surface.set_visible(visible);
        for popup in self.popups.lock().values() {
            popup.popup.set_visible(visible);
        }
    }

    fn restack_popups(&self) {
        if self.popups.is_empty() {
            return;
        }
        let stack = self.popup_display_stack.get();
        for popup in self.popups.lock().values() {
            if let Some(dl) = &*popup.display_link.borrow() {
                popup.popup.xdg.damage();
                stack.add_last_existing(dl);
            }
            popup.popup.xdg.restack_popups();
        }
        self.surface.client.state.tree_changed();
    }
}

object_base! {
    self = XdgSurface;
    version = self.base.version;
}

impl Object for XdgSurface {
    fn break_loops(&self) {
        self.ext.take();
        self.popups.clear();
        self.workspace.set(None);
    }
}

dedicated_add_obj!(XdgSurface, XdgSurfaceId, xdg_surfaces);

impl SurfaceExt for XdgSurface {
    fn node_layer(&self) -> NodeLayerLink {
        let Some(ext) = self.ext.get() else {
            return NodeLayerLink::Display;
        };
        ext.node_layer()
    }

    fn before_apply_commit(
        self: Rc<Self>,
        pending: &mut PendingState,
    ) -> Result<(), WlSurfaceError> {
        if !self.have_initial_commit.get()
            && let Some(ext) = self.ext.get()
        {
            ext.initial_configure()?;
            self.do_send_configure();
            self.have_initial_commit.set(true);
        }
        if let Some(pending) = &mut pending.xdg_surface
            && let Some(geometry) = pending.geometry.take()
        {
            let prev = self.geometry.replace(Some(geometry));
            if prev != Some(geometry) {
                self.update_extents();
                self.update_surface_position();
                if let Some(ext) = self.ext.get() {
                    ext.geometry_changed();
                }
            }
        }
        Ok(())
    }

    fn after_apply_commit(self: Rc<Self>) {
        if let Some(ext) = self.ext.get() {
            ext.post_commit();
        }
    }

    fn extents_changed(&self) {
        self.update_extents();
    }

    fn focus_node(&self) -> Option<Rc<dyn Node>> {
        self.ext.get()?.focus_node()
    }

    fn tray_item(self: Rc<Self>) -> Option<TrayItemId> {
        self.ext.get()?.tray_item()
    }
}

#[derive(Debug, Error)]
pub enum XdgSurfaceError {
    #[error(
        "Surface {0} cannot be turned into a xdg_surface because it already has an attached xdg_surface"
    )]
    AlreadyAttached(WlSurfaceId),
    #[error(transparent)]
    XdgPopupError(#[from] XdgPopupError),
    #[error("Surface {} cannot be assigned the role {} because it already has the role {}", .id, .new.name(), .old.name())]
    IncompatibleRole {
        id: XdgSurfaceId,
        old: XdgSurfaceRole,
        new: XdgSurfaceRole,
    },
    #[error(transparent)]
    ClientError(Box<ClientError>),
    #[error("Tried no set a non-positive width/height")]
    NonPositiveWidthHeight,
    #[error(
        "Cannot destroy xdg_surface {0} because it's associated xdg_toplevel/popup is not yet destroyed"
    )]
    RoleNotYetDestroyed(XdgSurfaceId),
    #[error("The surface still has popups attached")]
    PopupsNotYetDestroyed,
    #[error("The surface already has an assigned xdg_toplevel")]
    AlreadyConstructed,
    #[error(transparent)]
    WlSurfaceError(Box<WlSurfaceError>),
}
efrom!(XdgSurfaceError, WlSurfaceError);
efrom!(XdgSurfaceError, ClientError);
