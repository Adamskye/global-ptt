use std::{cell::RefCell, ops::Deref, rc::Rc, sync::mpsc};

use libpulse_binding::{
    callbacks::ListResult,
    context::{Context, FlagSet, State},
    error::PAErr,
    mainloop::standard::{IterateResult, Mainloop},
    operation,
    proplist::{Proplist, properties},
};

const VIRTUALMIC_DESCRIPTION: &str = "Global Push-to-Talk Virtual Microphone";
const VIRTUALMIC_NAME: &str = "GlobalPushToTalkVirtualMicrophone";

#[derive(Clone)]
pub struct PulseAudioState {
    mainloop: Rc<RefCell<Mainloop>>,
    context: Rc<RefCell<Context>>,
    src_name: Option<String>,
}

impl PulseAudioState {
    pub fn init() -> Result<Self, Error> {
        let mut proplist = Proplist::new().unwrap();
        proplist
            .set_str(properties::APPLICATION_NAME, "GlobalPushToTalk")
            .unwrap();

        let mainloop = Rc::new(RefCell::new(
            Mainloop::new().ok_or(Error::MainloopCreation)?,
        ));

        let context = Rc::new(RefCell::new(
            Context::new_with_proplist(
                mainloop.borrow().deref(),
                "GlobalPushToTalkContext",
                &proplist,
            )
            .ok_or(Error::ContextCreation)?,
        ));

        context.borrow_mut().connect(None, FlagSet::NOFLAGS, None)?;

        // Wait for context to be ready
        loop {
            match mainloop.borrow_mut().iterate(false) {
                IterateResult::Quit(_) | IterateResult::Err(_) => {
                    return Err(Error::MainloopTick);
                }
                IterateResult::Success(_) => {}
            }
            match context.borrow().get_state() {
                State::Ready => {
                    break;
                }
                State::Failed | State::Terminated => {
                    return Err(Error::ContextCreation);
                }
                _ => {}
            }
        }

        Ok(Self {
            mainloop: mainloop.clone(),
            context,
            src_name: None,
        })
    }

    pub fn remove_virtual_mic(&mut self) {
        let mut inner_introspect = self.context.borrow().introspect();

        let delete_op = self
            .context
            .borrow()
            .introspect()
            .get_module_info_list(move |item| match item {
                ListResult::Item(i) => {
                    if i.name.as_deref() != Some("module-remap-source") {
                        return;
                    }

                    if i.argument
                        .as_ref()
                        .filter(|args| args.contains(&format!("source_name={VIRTUALMIC_NAME}")))
                        .is_none()
                    {
                        return;
                    }

                    inner_introspect.unload_module(i.index, |_| {});
                }
                ListResult::End => {}
                ListResult::Error => {}
            });

        // wait for unloading to finish
        loop {
            match self.mainloop.borrow_mut().iterate(false) {
                IterateResult::Quit(_) | IterateResult::Err(_) => {
                    return;
                }
                _ => {}
            }
            if delete_op.get_state() == operation::State::Done {
                break;
            }
        }
    }

    pub fn set_virtual_mic(&mut self, source_name: &str) {
        // pactl load-module module-remap-source master=<mic name> source_name=<VIRTUALMIC_NAME> source_properties=device.description=<VIRTUALMIC_DESCRIPTION>

        self.remove_virtual_mic();

        let options = format!(
            "master={source_name} source_name={VIRTUALMIC_NAME} source_properties=\"device.description='{VIRTUALMIC_DESCRIPTION}'\""
        );

        let create_op =
            self.context
                .borrow()
                .introspect()
                .load_module("module-remap-source", &options, |_| {});

        // wait for loading to finish
        loop {
            match self.mainloop.borrow_mut().iterate(false) {
                IterateResult::Quit(_) | IterateResult::Err(_) => {
                    return;
                }
                _ => {}
            }
            if create_op.get_state() != operation::State::Running {
                let _ = self.set_mute(true);
                self.src_name = Some(source_name.to_string());
                return;
            }
        }
    }

    pub fn get_active_source(&self) -> Option<&str> {
        self.src_name.as_deref()
    }

    pub fn set_mute(&mut self, mute: bool) -> Result<(), Error> {
        let op =
            self.context
                .borrow()
                .introspect()
                .set_source_mute_by_name(VIRTUALMIC_NAME, mute, None);

        // wait for it to complete
        loop {
            match self.mainloop.borrow_mut().iterate(false) {
                IterateResult::Quit(_) | IterateResult::Err(_) => {
                    return Err(Error::MainloopTick);
                }
                _ => {}
            }
            if op.get_state() != operation::State::Running {
                return Ok(());
            }
        }
    }

    pub fn get_input_devices(&self) -> Vec<String> {
        let mut vec = Vec::new();
        let (tx, rx) = mpsc::channel();
        let op = self
            .context
            .borrow()
            .introspect()
            .get_source_info_list(move |item| {
                if let ListResult::Item(i) = item
                    && let Some(name) = &i.name
                    && name != VIRTUALMIC_NAME
                {
                    let _ = tx.send(name.to_string());
                }
            });

        loop {
            match self.mainloop.borrow_mut().iterate(false) {
                IterateResult::Success(_) => {}
                IterateResult::Quit(_) | IterateResult::Err(_) => return vec,
            }

            if op.get_state() != operation::State::Running {
                break;
            }
        }

        while let Ok(s) = rx.try_recv() {
            vec.push(s);
        }

        vec
    }
}

impl Drop for PulseAudioState {
    fn drop(&mut self) {
        self.context.borrow_mut().disconnect();
        self.mainloop
            .borrow_mut()
            .quit(libpulse_binding::def::Retval(0));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mainloop creation failed")]
    MainloopCreation,
    #[error("context creation failed")]
    ContextCreation,
    #[error("context connection failed: {0}")]
    ContextConnection(#[from] PAErr),
    #[error("failed to tick mainloop")]
    MainloopTick,
}
