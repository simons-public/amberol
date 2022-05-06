// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::RefCell;

use glib::clone;
use gst::prelude::*;
use gtk::{glib, subclass::prelude::*};

mod imp {
    use glib::{ParamFlags, ParamSpec, ParamSpecBoolean, Value};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct WaveformGenerator {
        pub uri: RefCell<Option<String>>,
        pub peaks: RefCell<Option<Vec<(f64, f64)>>>,
        pub pipeline: RefCell<Option<gst::Element>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WaveformGenerator {
        const NAME: &'static str = "WaveformGenerator";
        type Type = super::WaveformGenerator;
    }

    impl ObjectImpl for WaveformGenerator {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![ParamSpecBoolean::new(
                    "has-peaks",
                    "",
                    "",
                    false,
                    ParamFlags::READABLE,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &ParamSpec) -> Value {
            match pspec.name() {
                "has-peaks" => obj.peaks().is_some().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct WaveformGenerator(ObjectSubclass<imp::WaveformGenerator>);
}

impl Default for WaveformGenerator {
    fn default() -> Self {
        glib::Object::new(&[]).expect("Failed to create WaveformGenerator")
    }
}

impl WaveformGenerator {
    pub fn new() -> Self {
        WaveformGenerator::default()
    }

    pub fn set_uri(&self, uri: Option<String>) {
        self.imp().uri.replace(uri);
    }

    pub fn peaks(&self) -> Option<Vec<(f64, f64)>> {
        (*self.imp().peaks.borrow()).as_ref().cloned()
    }

    pub fn generate_peaks(&self) -> bool {
        if let Some(pipeline) = self.imp().pipeline.take() {
            // Stop any running pipeline, and ensure that we have nothing to
            // report
            self.imp().peaks.replace(None);
            pipeline.send_event(gst::event::Eos::new());
            match pipeline.set_state(gst::State::Null) {
                Ok(_) => {}
                Err(err) => warn!("Unable to set existing pipeline to Null state: {}", err),
            }
        }

        if self.imp().uri.borrow().is_none() {
            return false;
        }

        // Reset the peaks vector
        let peaks: Vec<(f64, f64)> = Vec::new();
        self.imp().peaks.replace(Some(peaks));

        let pipeline_str = "uridecodebin name=uridecodebin ! audioconvert ! audio/x-raw,channels=2 ! level name=level interval=250000000 ! fakesink name=faked";
        let pipeline = match gst::parse_launch(pipeline_str) {
            Ok(pipeline) => pipeline,
            Err(err) => {
                warn!("Unable to generate the waveform: {}", err);
                return false;
            }
        };

        let uridecodebin = pipeline
            .downcast_ref::<gst::Bin>()
            .unwrap()
            .by_name("uridecodebin")
            .unwrap();
        uridecodebin.set_property("uri", self.imp().uri.borrow().as_deref());

        let fakesink = pipeline
            .downcast_ref::<gst::Bin>()
            .unwrap()
            .by_name("faked")
            .unwrap();
        fakesink.set_property("qos", false);
        fakesink.set_property("sync", false);

        let bus = pipeline
            .bus()
            .expect("Pipeline without bus. Shouldn't happen!");

        debug!("Adding bus watch");
        bus.add_watch_local(clone!(@weak self as this, @weak pipeline => @default-return glib::Continue(false), move |_, msg| {
            use gst::MessageView;

            match msg.view() {
                MessageView::Eos(..) => {
                    debug!("End of waveform stream");
                    pipeline.set_state(gst::State::Null).expect("Unable to set 'null' state");
                    // We're done
                    this.imp().pipeline.replace(None);
                    this.notify("has-peaks");
                    return glib::Continue(false);
                }
                MessageView::Error(err) => {
                    warn!("Pipeline error: {:?}", err);
                    pipeline.set_state(gst::State::Null).expect("Unable to set 'null' state");
                    // We're done
                    this.imp().pipeline.replace(None);
                    this.notify("has-peaks");
                    return glib::Continue(false);
                }
                MessageView::Element(element) => {
                    if let Some(s) = element.structure() {
                        if s.has_name("level") {
                            let peaks_array = s.get::<&glib::ValueArray>("peak").unwrap();
                            let v1 = peaks_array[0].get::<f64>().unwrap();
                            let v2 = peaks_array[1].get::<f64>().unwrap();
                            // Normalize peaks between 0 and 1
                            let peak1 = f64::powf(10.0, v1 / 20.0);
                            let peak2 = f64::powf(10.0, v2 / 20.0);
                            if let Some(ref mut peaks) = *this.imp().peaks.borrow_mut() {
                                peaks.push((peak1, peak2));
                            }
                        }
                    }
                }
                _ => (),
            };

            glib::Continue(true)
        }))
        .expect("failed to add bus watch");

        match pipeline.set_state(gst::State::Playing) {
            Ok(_) => {
                self.imp().pipeline.replace(Some(pipeline));
                true
            }
            Err(err) => {
                warn!("Unable to generate the waveform: {}", err);
                pipeline
                    .set_state(gst::State::Null)
                    .expect("Pipeline reset failed");
                self.imp().peaks.replace(None);
                self.notify("has-peaks");
                false
            }
        }
    }
}
