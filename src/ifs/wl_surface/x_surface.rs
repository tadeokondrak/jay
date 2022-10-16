use {
    crate::{
        ifs::wl_surface::{x_surface::xwindow::Xwindow, SurfaceExt, WlSurface, WlSurfaceError},
        leaks::Tracker,
        tree::ToplevelNode,
        utils::clonecell::CloneCell,
        xwayland::XWaylandEvent,
    },
    std::rc::Rc,
};

pub mod xwindow;

pub struct XSurface {
    pub surface: Rc<WlSurface>,
    pub xwindow: CloneCell<Option<Rc<Xwindow>>>,
    pub tracker: Tracker<Self>,
}

impl SurfaceExt for XSurface {
    fn post_commit(self: Rc<Self>) {
        if let Some(xwindow) = self.xwindow.get() {
            xwindow.map_status_changed();
        }
    }

    fn on_surface_destroy(&self) -> Result<(), WlSurfaceError> {
        self.surface.unset_ext();
        if let Some(xwindow) = self.xwindow.take() {
            xwindow.tl_destroy();
            xwindow.data.window.set(None);
            xwindow.data.surface_id.set(None);
            xwindow
                .data
                .state
                .xwayland
                .queue
                .push(XWaylandEvent::SurfaceDestroyed(self.surface.id));
        }
        Ok(())
    }

    fn extents_changed(&self) {
        if let Some(xwindow) = self.xwindow.get() {
            xwindow.toplevel_data.pos.set(self.surface.extents.get());
            xwindow.tl_extents_changed();
        }
    }

    fn into_xsurface(self: Rc<Self>) -> Option<Rc<XSurface>> {
        Some(self)
    }
}
