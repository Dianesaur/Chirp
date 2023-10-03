mod imp {
    use adw::subclass::prelude::*;
    use adw::ApplicationWindow;
    use gio::ListStore;
    use glib::subclass::InitializingObject;
    use glib::{object_subclass, Binding};
    use gtk::{
        gio, glib, Button, CompositeTemplate, Label, ListBox, Revealer, ScrolledWindow, Stack,
        TextView,
    };
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;

    use crate::user::UserObject;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/com/github/therustypickle/chirp/window.xml")]
    pub struct Window {
        #[template_child]
        pub message_scroller: TemplateChild<ScrolledWindow>,
        #[template_child]
        pub message_entry: TemplateChild<TextView>,
        #[template_child]
        pub message_list: TemplateChild<ListBox>,
        #[template_child]
        pub send_button: TemplateChild<Button>,
        #[template_child]
        pub user_list: TemplateChild<ListBox>,
        #[template_child]
        pub stack: TemplateChild<Stack>,
        #[template_child]
        pub my_profile: TemplateChild<Button>,
        #[template_child]
        pub new_chat: TemplateChild<Button>,
        #[template_child]
        pub placeholder: TemplateChild<Label>,
        #[template_child]
        pub entry_revealer: TemplateChild<Revealer>,
        pub users: OnceCell<ListStore>,
        pub chatting_with: Rc<RefCell<Option<UserObject>>>,
        pub own_profile: Rc<RefCell<Option<UserObject>>>,
        pub last_selected_user: Cell<i32>,
        pub bindings: RefCell<Vec<Binding>>,
    }

    #[object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "MainWindow";
        type Type = super::Window;
        type ParentType = ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_callbacks();
            obj.setup_users();
            obj.setup_actions();
        }
    }

    impl WindowImpl for Window {}

    impl WidgetImpl for Window {}

    impl ApplicationWindowImpl for Window {}

    impl AdwApplicationWindowImpl for Window {}
}

use std::time::Duration;

use adw::subclass::prelude::*;
use adw::{prelude::*, Application};
use chrono::{Local, DateTime};
use gio::{ActionGroup, ActionMap, ListStore, SimpleAction};
use glib::{clone, timeout_add_local_once, wrapper, ControlFlow, Object, Receiver};
use gtk::{
    gio, glib, Accessible, ApplicationWindow, Buildable, ConstraintTarget, ListBox, ListBoxRow,
    Native, Root, ShortcutManager, Widget,
};
use tracing::info;

use crate::message::{MessageObject, MessageRow};
use crate::user::{UserObject, UserProfile, UserPrompt, UserRow};
use crate::utils::generate_random_avatar_link;
use crate::ws::{FullUserData, MessageData, RequestType, UserIDs};

wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, ApplicationWindow, gtk::Window, Widget,
        @implements ActionGroup, ActionMap, Accessible, Buildable,
                    ConstraintTarget, Native,Root, ShortcutManager;
}

impl Window {
    pub fn new(app: &Application) -> Self {
        Object::builder().property("application", app).build()
    }

    fn setup_callbacks(&self) {
        let imp = self.imp();
        imp.stack.set_visible_child_name("main");

        imp.user_list
            .connect_row_activated(clone!(@weak self as window => move |listbox, row| {
                let last_index = window.imp().last_selected_user.get();
                let index = row.index();

                if last_index != index {
                    window.remove_avatar_css(last_index, listbox);
                    window.add_avatar_css(index, listbox);
                }

                let selected_chat = window.get_users_liststore()
                .item(index as u32)
                .unwrap()
                .downcast::<UserObject>()
                .unwrap();

                info!("Selected a new User from list");

                window.imp().last_selected_user.set(index);
                window.set_chatting_with(selected_chat);
                window.remove_last_binding();
                window.bind();
            }));

        self.imp()
            .new_chat
            .connect_clicked(clone!(@weak self as window => move |_| {
                let prompt = UserPrompt::new("Start Chat").add_user(&window);
                prompt.present();
            }));

        self.imp()
            .my_profile
            .connect_clicked(clone!(@weak self as window => move |_| {
                UserProfile::new(window.get_chatting_from(), &window, true);
            }));

        self.imp()
            .send_button
            .connect_clicked(clone!(@weak self as window => move |_| {
                window.send_message();
                window.grab_focus();
            }));

        let scroller_bar = self.imp().message_scroller.get();
        let vadjust = scroller_bar.vadjustment();
        vadjust.connect_changed(clone!(@weak vadjust => move |adjust| {
            let upper = adjust.upper();
            vadjust.set_value(upper);
        }));

        self.imp().message_entry.get().buffer().connect_changed(
            clone!(@weak self as window => move |buffer| {
                let char_count = buffer.char_count();
                let should_be_enabled = char_count != 0;
                window.imp().send_button.set_sensitive(should_be_enabled);
                if should_be_enabled {
                    window.imp().placeholder.set_visible(false);
                } else {
                    window.imp().placeholder.set_visible(true);
                }

            }),
        );

        let window = self.clone();
        timeout_add_local_once(Duration::from_millis(500), move || {
            window.imp().entry_revealer.set_reveal_child(true);
        });
    }

    fn setup_actions(&self) {
        let button_action = SimpleAction::new("send-message", None);
        button_action.connect_activate(clone!(@weak self as window => move |_, _| {
            window.send_message();
            window.grab_focus();
        }));

        self.add_action(&button_action);
    }

    fn bind(&self) {
        let mut bindings = self.imp().bindings.borrow_mut();
        let chatting_with = self.get_chatting_with();
        let title_binding = chatting_with
            .bind_property("name", self, "title")
            .transform_to(|_, name: String| Some(format!("Chirp - {}", name)))
            .sync_create()
            .build();
        bindings.push(title_binding);
    }

    fn remove_last_binding(&self) {
        let last_binding = self.imp().bindings.borrow_mut().pop().unwrap();
        last_binding.unbind();
    }

    fn setup_users(&self) {
        let users = ListStore::new::<UserObject>();
        self.imp().users.set(users).expect("Could not set users");
        self.imp().last_selected_user.set(0);

        let data: UserObject = self.create_owner("Me");

        let user_clone_1 = data.clone();
        let user_clone_2 = data.clone();

        info!("Setting own profile");
        self.imp().own_profile.replace(Some(data));
        let user_row = UserRow::new(user_clone_1);
        user_row.imp().user_avatar.add_css_class("user-selected");

        let user_list_row = ListBoxRow::builder()
            .child(&user_row)
            .activatable(true)
            .selectable(false)
            .can_focus(false)
            .build();

        self.get_user_list().append(&user_list_row);
        self.set_chatting_with(user_clone_2);

        if let Some(row) = self.get_user_list().row_at_index(0) {
            self.get_user_list().select_row(Some(&row));
        }
        self.bind();
    }

    pub fn get_chatting_with(&self) -> UserObject {
        self.imp()
            .chatting_with
            .borrow()
            .clone()
            .expect("Expected an UserObject")
    }

    fn set_chatting_with(&self, user: UserObject) {
        info!("Setting chatting with {}", user.name());
        user.add_queue_to_first(RequestType::NewUserSelection(user.clone()));
        let message_list = user.messages();
        self.imp().message_list.bind_model(
            Some(&message_list),
            clone!(@weak self as window => @default-panic, move |obj| {
                let message_data = obj.downcast_ref().expect("No MessageObject here");
                let row = window.get_message_row(message_data);
                window.grab_focus();
                row.upcast()
            }),
        );
        self.imp().chatting_with.replace(Some(user));
    }

    pub fn get_chatting_from(&self) -> UserObject {
        self.imp()
            .own_profile
            .borrow()
            .clone()
            .expect("Expected an UserObject")
    }

    pub fn get_owner_id(&self) -> u64 {
        self.get_chatting_from().user_id()
    }

    fn get_users_liststore(&self) -> ListStore {
        self.imp()
            .users
            .get()
            .expect("User liststore is not set")
            .clone()
    }

    fn chatting_with_messages(&self) -> ListStore {
        self.get_chatting_with().messages()
    }

    fn send_message(&self) {
        let buffer = self.imp().message_entry.buffer();
        let content = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .trim()
            .to_string();

        if content.is_empty() {
            info!("Empty text found");
            return;
        }

        info!("Sending new text message: {}", content);

        let sender = self.get_chatting_from();
        let receiver = self.get_chatting_with();

        let receiver_id = receiver.user_id();
        let current_time = Local::now();
        let created_at_naive = current_time.naive_local().to_string();

        let created_at = current_time.to_string();

        let message = MessageObject::new(
            content.to_owned(),
            true,
            sender.clone(),
            receiver,
            created_at_naive,
        );

        let send_message_data = MessageData::new_incomplete(created_at, receiver_id, content);

        sender.add_to_queue(RequestType::SendMessage(send_message_data, message.clone()));

        buffer.set_text("");

        self.chatting_with_messages().append(&message);
    }

    fn receive_message(&self, message_data: MessageData, sender: UserObject) {
        info!(
            "Receiving message from {} with content: {}",
            sender.name(),
            message_data.message
        );
        let receiver = self.get_chatting_with();

        // NOTE temporary solution. Later when user timezone is saved on the server side, it should
        // send the correct time without zone  
        let parsed_date_time = DateTime::parse_from_str(&message_data.created_at, "%Y-%m-%d %H:%M:%S%.3f %z").unwrap().naive_local().to_string();

        let message = MessageObject::new(
            message_data.message,
            false,
            sender.clone(),
            receiver,
            parsed_date_time,
        );

        sender.messages().append(&message);
    }

    fn get_message_row(&self, data: &MessageObject) -> ListBoxRow {
        let message_row = MessageRow::new(data.clone(), self);
        ListBoxRow::builder()
            .child(&message_row)
            .selectable(false)
            .activatable(false)
            .can_focus(false)
            .build()
    }

    fn create_owner(&self, name: &str) -> UserObject {
        // It's a new user + owner so the ID will be generated on the server side
        let user_data = UserObject::new(name, Some(generate_random_avatar_link()), None, None);
        user_data.handle_ws(self.clone());
        self.get_users_liststore().append(&user_data);

        user_data
    }

    pub fn handle_ws_message(&self, user: &UserObject, receiver: Receiver<String>) {
        receiver.attach(None, clone!(@weak user as user_object, @weak self as window => @default-return ControlFlow::Break, move |response| {
            let response_data: Vec<&str> = response.splitn(2, ' ').collect();
            match response_data[0] {
                "/get-user-data" | "/new-user-message" => {
                    let user_data = FullUserData::from_json(response_data[1]);
                    let user = window.create_user(user_data);
                    let user_row = UserRow::new(user);
                    user_row.imp().user_avatar.add_css_class("user-inactive");
                    let user_list_row = ListBoxRow::builder()
                        .child(&user_row)
                        .activatable(true)
                        .selectable(false)
                        .can_focus(false)
                        .build();
                    window.get_user_list().append(&user_list_row);
                }
                "/message" => {
                    let message_data = MessageData::from_json(response_data[1]);
                    window.receive_message(message_data, user_object)},
                "/update-user-id" => {
                    let chatting_from = window.get_chatting_from();
                    if user_object == chatting_from {
                        let id_data = UserIDs::from_json(response_data[1]);
                        chatting_from.set_owner_id(id_data.user_id);
                    }
                    chatting_from.add_to_queue(RequestType::UpdateIDs);
                }
                _ => {}
            }
            ControlFlow::Continue
        }));
    }

    fn create_user(&self, user_data: FullUserData) -> UserObject {
        info!(
            "Creating new user with name: {}, id: {}",
            user_data.user_name, user_data.user_id
        );

        let new_user_data = UserObject::new(
            &user_data.user_name,
            user_data.image_link,
            Some(&self.get_owner_name_color()),
            Some(user_data.user_id),
        );

        if user_data.message.is_some() {
            self.receive_message(user_data.message.unwrap(), new_user_data.clone())
        }

        // Every single user in the UserList of the client will have the owner User ID for reference
        let chatting_from = self.get_chatting_from();
        chatting_from
            .bind_property("user-id", &new_user_data, "owner-id")
            .sync_create()
            .build();

        chatting_from
            .bind_property("user-token", &new_user_data, "user-token")
            .sync_create()
            .build();

        new_user_data.handle_ws(self.clone());
        self.get_users_liststore().append(&new_user_data);
        new_user_data
    }

    fn get_user_list(&self) -> ListBox {
        self.imp().user_list.get()
    }

    fn get_owner_name_color(&self) -> String {
        self.get_chatting_from().name_color()
    }

    fn grab_focus(&self) {
        self.imp().message_entry.grab_focus();
    }

    fn remove_avatar_css(&self, index: i32, listbox: &ListBox) {
        let b = listbox.row_at_index(index).unwrap();
        let c: UserRow = b.child().unwrap().downcast().unwrap();

        c.imp().user_avatar.remove_css_class("user-selected");
        c.imp().user_avatar.add_css_class("user-inactive");
    }

    fn add_avatar_css(&self, index: i32, listbox: &ListBox) {
        let b = listbox.row_at_index(index).unwrap();
        let c: UserRow = b.child().unwrap().downcast().unwrap();

        c.imp().user_avatar.add_css_class("user-selected");
        c.imp().user_avatar.remove_css_class("user-inactive");
    }
}
