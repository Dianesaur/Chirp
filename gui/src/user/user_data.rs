mod imp {
    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gdk::Paintable;
    use gio::ListStore;
    use glib::{derived_properties, object_subclass, Properties};
    use gtk::{gdk, glib};
    use std::cell::{Cell, OnceCell, RefCell};

    use super::UserData;
    use crate::ws::RequestType;
    use crate::ws::WSObject;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::UserObject)]
    pub struct UserObject {
        #[property(name = "user-id", get, set, type = u64, member = user_id)]
        #[property(name = "big-image", get, set, type = Option<Paintable>, member = big_image)]
        #[property(name = "small-image", get, set, type = Option<Paintable>, member = small_image)]
        #[property(name = "name", get, set, type = String, member = name)]
        #[property(name = "name-color", get, set, type = String, member = name_color)]
        #[property(name = "image-link", get, set, type = Option<String>, member = image_link)]
        pub data: RefCell<UserData>,
        #[property(get, set)]
        pub messages: OnceCell<ListStore>,
        #[property(get, set)]
        pub user_ws: OnceCell<WSObject>,
        // TODO mutex + gio::spawn_blocking to prevent borrow_mut colliding?
        pub request_queue: RefCell<Vec<RequestType>>,
        #[property(get, set)]
        pub request_processing: Cell<bool>,
        #[property(get, set)]
        pub owner_id: Cell<u64>,
        #[property(get, set)]
        pub user_token: OnceCell<String>,
    }

    #[object_subclass]
    impl ObjectSubclass for UserObject {
        const NAME: &'static str = "UserObject";
        type Type = super::UserObject;
    }

    #[derived_properties]
    impl ObjectImpl for UserObject {}
}

use adw::prelude::*;
use gdk::{gdk_pixbuf, Paintable, Texture};
use gdk_pixbuf::InterpType;
use gio::subclass::prelude::ObjectSubclassIsExt;
use gio::{spawn_blocking, ListStore};
use glib::{
    clone, closure_local, timeout_add_seconds_local_once, Bytes, ControlFlow, MainContext, Object,
    Priority, Receiver, Sender,
};
use gtk::{gdk, glib, Image};
use tracing::{debug, info};

use crate::message::MessageObject;
use crate::utils::{generate_random_avatar_link, get_avatar, get_random_color};
use crate::window::Window;
use crate::ws::{
    FullUserData, GetUserData, ImageUpdate, NameUpdate, RequestType, UserIDs, WSObject,
};

glib::wrapper! {
    pub struct UserObject(ObjectSubclass<imp::UserObject>);
}

impl UserObject {
    pub fn new(
        name: &str,
        image_link: Option<String>,
        color_to_ignore: Option<&str>,
        user_id: Option<u64>,
    ) -> Self {
        let ws = WSObject::new();
        let messages = ListStore::new::<MessageObject>();
        let random_color = get_random_color(color_to_ignore);

        let id = if let Some(id) = user_id { id } else { 0 };

        let obj: UserObject = Object::builder()
            .property("user-id", id)
            .property("name", name)
            .property("image-link", image_link.clone())
            .property("messages", messages)
            .property("name-color", random_color)
            .build();

        obj.check_image_link();
        obj.set_user_ws(ws);

        let user_object = obj.clone();

        // This signal gets emitted when the connection is once lost but reconnected again
        // TODO use a separate signal to handle the reconnection
        // then do another signal to resume queueing
        obj.user_ws().connect_closure(
            "ws-reconnect",
            false,
            closure_local!(move |_from: WSObject, _success: bool| {
                let old_queue = user_object.imp().request_queue.borrow().clone();
                // As the server lost it's previous data, the reconnection must be done
                // first before any other pending request can be processed.
                // So reconnect -> process previous pending requests
                user_object.imp().request_queue.replace(Vec::new());
                user_object.add_to_queue(RequestType::ReconnectUser);

                // TODO add a function to add using vector
                // 2 second wait time so the reconnection can happen before older requests can be processed
                timeout_add_seconds_local_once(
                    2,
                    clone!(@weak user_object => move || {
                        for old in old_queue {
                            user_object.add_to_queue(old);
                        }
                    }),
                );
            }),
        );
        obj
    }

    // TODO: Pass a result instead of Bytes directly
    fn check_image_link(&self) {
        if let Some(image_link) = self.image_link() {
            let (sender, receiver) = MainContext::channel(Priority::default());
            self.set_user_image(receiver);
            spawn_blocking(move || {
                let avatar = get_avatar(image_link);
                sender.send(avatar).unwrap();
            });
        }
    }

    // TODO: Verify image link
    #[allow(deprecated)]
    fn set_user_image(&self, receiver: Receiver<Bytes>) {
        receiver.attach(
            None,
            clone!(@weak self as user_object => @default-return ControlFlow::Break,
                move |image_data| {
                    let texture = Texture::from_bytes(&image_data).unwrap();
                    let pixbuf = gdk::pixbuf_get_from_texture(&texture).unwrap();

                    let big_image_buf = pixbuf.scale_simple(150, 150, InterpType::Hyper).unwrap();
                    let small_image_buf = pixbuf.scale_simple(45, 45, InterpType::Hyper).unwrap();

                    let big_image = Image::from_pixbuf(Some(&big_image_buf));
                    let small_image = Image::from_pixbuf(Some(&small_image_buf));

                    let paintable = big_image.paintable().unwrap();
                    user_object.set_big_image(paintable);

                    let paintable = small_image.paintable().unwrap();
                    user_object.set_small_image(paintable);
                    ControlFlow::Break
                }
            ),
        );
    }

    /// Adds stuff to queue and start the process to process them
    pub fn add_to_queue(&self, request_type: RequestType) -> &Self {
        {
            let mut queue = self.imp().request_queue.borrow_mut();
            queue.push(request_type);
        }

        // The process must not start twice otherwise the same
        // request can get processed twice, creating disaster
        if !self.request_processing() {
            self.process_queue();
        };
        self
    }

    /// Processes queued stuff if ws conn is available
    fn process_queue(&self) {
        self.set_request_processing(true);

        let user_ws = self.user_ws();

        let queue_list = self.imp().request_queue.borrow().clone();

        let mut highest_index = 0;
        for task in queue_list.iter() {
            if user_ws.ws_conn().is_some() {
                debug!("starting processing {task:?}");
                match task {
                    RequestType::ReconnectUser => {
                        let id_data = UserIDs::new_json(self);
                        user_ws.reconnect_user(id_data);
                    }
                    RequestType::UpdateIDs => {
                        let id_data = UserIDs::new_json(self);
                        user_ws.update_ids(id_data)
                    }
                    RequestType::CreateNewUser => {
                        let user_data = FullUserData::new_json(self);
                        user_ws.create_new_user(user_data);
                    }
                    // Already using MessageData -> String
                    RequestType::SendMessage(msg) => user_ws.send_text_message(&msg),

                    RequestType::ImageUpdated(link) => {
                        let image_data = ImageUpdate::new_json(link.to_string(), self.user_token());
                        user_ws.image_link_updated(&image_data);
                    }
                    RequestType::NameUpdated(name) => {
                        let name_data = NameUpdate::new_json(name.to_string(), self.user_token());
                        user_ws.name_updated(&name_data)
                    }
                    RequestType::GetUserData(id) => {
                        let user_data = GetUserData::new_json(id.to_owned(), self.user_token());
                        user_ws.get_user_data(&user_data)
                    }
                }
                highest_index += 1;
            } else {
                info!("Connection lost. Stopping processing request");
                break;
            }
        }

        // Remove the processed requests
        {
            let mut queue_list = self.imp().request_queue.borrow_mut();
            for _x in 0..highest_index {
                queue_list.remove(0);
            }
        }

        // TODO do not stop processing if the connection failed
        self.set_request_processing(false);
    }

    pub fn set_new_name(&self, name: String) {
        self.set_name(name);
    }

    pub fn set_new_image_link(&self, link: String) {
        self.set_image_link(link);
        self.check_image_link()
    }

    pub fn set_random_image(&self) {
        let new_link = generate_random_avatar_link();
        info!("Generated random image link: {}", new_link);
        self.add_to_queue(RequestType::ImageUpdated(new_link.to_owned()));
        self.set_new_image_link(new_link);
    }

    pub fn handle_ws(&self, window: Window) {
        let user_object = self.clone();

        let user_ws = self.user_ws();
        user_ws.connect_closure(
            "ws-success",
            false,
            closure_local!(move |_from: WSObject, _success: bool| {
                let (sender, receiver) = MainContext::channel(Priority::DEFAULT);
                user_object.start_listening(sender.clone());
                window.handle_ws_message(&user_object, receiver);
            }),
        );
    }

    fn start_listening(&self, sender: Sender<String>) {
        let user_ws = self.user_ws();
        if !user_ws.is_reconnecting() {
            if self.user_id() == 0 {
                self.add_to_queue(RequestType::CreateNewUser);
            } else {
                self.add_to_queue(RequestType::UpdateIDs);
            }
        }

        let id = user_ws.ws_conn().unwrap().connect_message(
            clone!(@weak self as user_object => move |_ws, _s, bytes| {
                let byte_slice = bytes.to_vec();
                let text = String::from_utf8(byte_slice).unwrap();
                debug!("{} Received from WS: {text}", user_object.name());

                if text.starts_with('/') {
                    //info!("Current UserToken for {} is {}. Owner ID {}", user_object.user_id(), user_object.user_token(), user_object.owner_id());
                    let splitted_data: Vec<&str> = text.splitn(2, ' ').collect();
                    match splitted_data[0] {
                        "/update-user-id" => {
                            let id_data = UserIDs::from_json(splitted_data[1]);
                            user_object.set_user_id(id_data.user_id);
                            user_object.set_user_token(id_data.user_token);
                            sender.send(text).unwrap();
                        }
                        "/image-updated" => {
                            user_object.set_image_link(splitted_data[1]);
                            user_object.check_image_link();
                        },
                        "/name-updated" => user_object.set_name(splitted_data[1]),
                        "/message" | "/get-user-data" | "/new-user-message" => sender.send(text).unwrap(),
                        _ => {}
                    }
                }
            }),
        );

        self.user_ws().set_signal_id(id);
    }
}

#[derive(Default, Clone)]
pub struct UserData {
    pub user_id: u64,
    pub name: String,
    pub name_color: String,
    pub big_image: Option<Paintable>,
    pub small_image: Option<Paintable>,
    pub image_link: Option<String>,
}
