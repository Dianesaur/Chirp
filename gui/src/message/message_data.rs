mod imp {
    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use glib::{derived_properties, object_subclass, Properties};
    use gtk::glib;
    use std::cell::{OnceCell, RefCell};

    use super::MessageData;
    use crate::message::MessageRow;
    use crate::user::UserObject;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::MessageObject)]
    pub struct MessageObject {
        #[property(name = "message", get, set, type = String, member = message)]
        #[property(name = "is-send", get, set, type = bool, member = is_send)]
        pub data: RefCell<MessageData>,
        #[property(get, set)]
        pub sent_from: OnceCell<UserObject>,
        #[property(get, set)]
        pub sent_to: OnceCell<UserObject>,
        #[property(get, set)]
        pub created_at: OnceCell<String>,
        #[property(get, set)]
        pub message_number: OnceCell<u64>,
        #[property(get, set)]
        pub target_row: RefCell<Option<MessageRow>>,
    }

    #[object_subclass]
    impl ObjectSubclass for MessageObject {
        const NAME: &'static str = "MessageObject";
        type Type = super::MessageObject;
    }

    #[derived_properties]
    impl ObjectImpl for MessageObject {}
}

use glib::{wrapper, Object};
use gtk::glib;

use crate::user::UserObject;

wrapper! {
    pub struct MessageObject(ObjectSubclass<imp::MessageObject>);
}

impl MessageObject {
    pub fn new(
        message: String,
        is_send: bool,
        sent_from: UserObject,
        sent_to: UserObject,
        created_at: String,
        message_number: Option<u64>,
    ) -> Self {
        let obj: MessageObject = Object::builder()
            .property("is-send", is_send)
            .property("message", message)
            .property("sent-from", sent_from)
            .property("sent-to", sent_to)
            .property("created-at", created_at)
            .build();

        if let Some(num) = message_number {
            obj.set_message_number(num)
        }

        obj
    }
}

#[derive(Default, Clone)]
pub struct MessageData {
    pub message: String,
    pub is_send: bool,
}
