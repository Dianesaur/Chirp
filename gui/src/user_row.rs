mod imp {
    use adw::{subclass::prelude::*, Avatar};
    use glib::subclass::InitializingObject;
    use glib::{object_subclass, Binding};
    use gtk::{glib, Box, CompositeTemplate, Label, Popover};
    use std::cell::{Cell, OnceCell, RefCell};

    use crate::user_data::UserObject;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/com/github/therustypickle/chirp/user_row.xml")]
    pub struct UserRow {
        #[template_child]
        pub user_avatar: TemplateChild<Avatar>,
        #[template_child]
        pub user_popover: TemplateChild<Popover>,
        #[template_child]
        pub popover_label: TemplateChild<Label>,
        pub popover_visible: Cell<bool>,
        pub bindings: RefCell<Vec<Binding>>,
        pub user_data: OnceCell<UserObject>,
    }

    #[object_subclass]
    impl ObjectSubclass for UserRow {
        // `NAME` needs to match `class` attribute of template
        const NAME: &'static str = "UserRow";
        type Type = super::UserRow;
        type ParentType = Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    // Trait shared by all GObjects
    impl ObjectImpl for UserRow {}

    // Trait shared by all widgets
    impl WidgetImpl for UserRow {}

    // Trait shared by all boxes
    impl BoxImpl for UserRow {}
}

use crate::user_data::UserObject;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gio::glib::closure_local;
use glib::{wrapper, Object};
use gtk::gdk::Paintable;
use gtk::prelude::*;
use gtk::{glib, Accessible, Box, Buildable, ConstraintTarget, Orientable, Widget};
use tracing::info;

wrapper! {
    pub struct UserRow(ObjectSubclass<imp::UserRow>)
    @extends Box, Widget,
    @implements Accessible, Buildable, ConstraintTarget, Orientable;
}

impl UserRow {
    pub fn new(object: UserObject) -> Self {
        let row: UserRow = Object::builder().build();
        row.imp().popover_visible.set(false);

        let avatar = row.imp().user_avatar.get();

        let row_clone = row.clone();

        object.connect_closure(
            "updating-image",
            false,
            closure_local!(move |from: UserObject, status: Paintable| {
                info!("Updating image for avatar {} on UserRow", from.name());
                let avatar = row_clone.imp().user_avatar.get();
                avatar.set_custom_image(Some(&status))
            }),
        );

        let motion = gtk::EventControllerMotion::new();
        avatar.add_controller(motion.clone());

        let row_clone = row.clone();

        motion.connect_enter(move |_, _, _| {
            if !row_clone.imp().popover_visible.get() {
                let popover = row_clone.imp().user_popover.get();
                let avatar = row_clone.imp().user_avatar.get();

                let popover_text = row_clone.imp().user_data.get().unwrap().name();

                let position = avatar.allocation();

                let x_position = position.x() + 40;
                let y_position = position.y() + 20;

                let position = gtk::gdk::Rectangle::new(x_position, y_position, -1, -1);

                popover.set_pointing_to(Some(&position));
                row_clone.imp().popover_label.set_label(&popover_text);

                row_clone.imp().user_popover.get().set_visible(true);
                row_clone.imp().popover_visible.set(true);
            }
        });

        let row_clone = row.clone();
        motion.connect_leave(move |_| {
            if row_clone.imp().popover_visible.get() {
                row_clone.imp().user_popover.get().set_visible(false);
                row_clone.imp().popover_visible.set(false);
            }
        });

        row.imp().user_data.set(object).unwrap();
        row
    }

    pub fn bind(&self) {
        let mut bindings = self.imp().bindings.borrow_mut();
        let user_avatar = self.imp().user_avatar.get();

        let user_object = self.imp().user_data.get().unwrap();

        let image_available = user_object.image();

        let avatar_text_binding = user_object
            .bind_property("name", &user_avatar, "text")
            .sync_create()
            .build();

        if image_available.is_some() {
            let avatar_image_binding = user_object
                .bind_property("image", &user_avatar, "custom-image")
                .sync_create()
                .build();
            bindings.push(avatar_image_binding);
        }
        bindings.push(avatar_text_binding);
    }
}