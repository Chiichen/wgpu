//! This module contains common test-only code that needs to be shared between the examples and the tests.
#![allow(dead_code)] // This module is used in a lot of contexts and only parts of it will be used

use std::panic::{catch_unwind, AssertUnwindSafe};

use wgt::{BackendBit, DeviceDescriptor, DownlevelCapabilities, Features, Limits};

use wgpu::{util, Adapter, Device, Instance, Queue};

pub mod image;

async fn initialize_device(
    adapter: &Adapter,
    features: Features,
    limits: Limits,
) -> (Device, Queue) {
    let bundle = adapter
        .request_device(
            &DeviceDescriptor {
                label: None,
                features,
                limits,
            },
            None,
        )
        .await;

    match bundle {
        Ok(b) => b,
        Err(e) => panic!("Failed to initialize device: {}", e),
    }
}

pub struct TestingContext {
    pub adapter: Adapter,
    pub adapter_info: wgt::AdapterInfo,
    pub device: Device,
    pub queue: Queue,
}

// A rather arbitrary set of limits which should be lower than all devices wgpu reasonably expects to run on and provides enough resources for most tests to run.
// Adjust as needed if they are too low/high.
pub fn lowest_reasonable_limits() -> Limits {
    Limits {
        max_texture_dimension_1d: 1024,
        max_texture_dimension_2d: 1024,
        max_texture_dimension_3d: 32,
        max_texture_array_layers: 32,
        max_bind_groups: 2,
        max_dynamic_uniform_buffers_per_pipeline_layout: 2,
        max_dynamic_storage_buffers_per_pipeline_layout: 2,
        max_sampled_textures_per_shader_stage: 2,
        max_samplers_per_shader_stage: 2,
        max_storage_buffers_per_shader_stage: 2,
        max_storage_textures_per_shader_stage: 2,
        max_uniform_buffers_per_shader_stage: 2,
        max_uniform_buffer_binding_size: 256,
        max_storage_buffer_binding_size: 1 << 16,
        max_vertex_buffers: 4,
        max_vertex_attributes: 4,
        max_vertex_buffer_array_stride: 32,
        max_push_constant_size: 0,
    }
}

fn lowest_downlevel_properties() -> DownlevelCapabilities {
    DownlevelCapabilities {
        flags: wgt::DownlevelFlags::empty(),
        shader_model: wgt::ShaderModel::Sm2,
    }
}

// This information determines if a test should run.
pub struct TestParameters {
    pub required_features: Features,
    pub required_limits: Limits,
    pub required_downlevel_properties: DownlevelCapabilities,
    // Backends where test should fail.
    pub failures: Vec<(Option<wgpu::BackendBit>, Option<usize>, Option<String>)>,
}

impl Default for TestParameters {
    fn default() -> Self {
        Self {
            required_features: Features::empty(),
            required_limits: lowest_reasonable_limits(),
            required_downlevel_properties: lowest_downlevel_properties(),
            failures: Vec::new(),
        }
    }
}

bitflags::bitflags! {
    pub struct FailureReason: u8 {
        const BACKEND = 0x1;
        const VENDOR = 0x2;
        const ADAPTER = 0x4;
        const ALWAYS = 0x8;
    }
}

// Builder pattern to make it easier
impl TestParameters {
    /// Set of common features that most tests require.
    pub fn test_features(self) -> Self {
        self.features(Features::MAPPABLE_PRIMARY_BUFFERS | Features::VERTEX_WRITABLE_STORAGE)
    }

    /// Set the list of features this test requires.
    pub fn features(mut self, features: Features) -> Self {
        self.required_features |= features;
        self
    }

    /// Set the list 
    pub fn limits(mut self, limits: Limits) -> Self {
        self.required_limits = limits;
        self
    }

    /// Mark the test as always failing, equivilant to specific_failure(None, None, None)
    pub fn failure(mut self) -> Self {
        self.failures.push((None, None, None));
        self
    }

    /// Mark the test as always failing on a specific backend, equivilant to specific_failure(backend, None, None)
    pub fn backend_failures(mut self, backends: wgpu::BackendBit) -> Self {
        self.failures.push((Some(backends), None, None));
        self
    }

    /// Determines if a test should fail under a particular set of conditions. If any of these are None, that means that it will match anything in that field.
    ///
    /// ex.
    /// `specific_failure(Some(wgpu::BackendBit::DX11 | wgpu::BackendBit::DX12), None, Some("RTX"))`
    /// means that this test will fail on all cards with RTX in their name on either D3D backend, no matter the vendor ID.
    pub fn specific_failure(
        mut self,
        backends: Option<BackendBit>,
        vendor: Option<usize>,
        device: Option<&'static str>,
    ) -> Self {
        self.failures.push((
            backends,
            vendor,
            device.as_ref().map(AsRef::as_ref).map(str::to_lowercase),
        ));
        self
    }
}

pub fn initialize_test(parameters: TestParameters, test_function: impl FnOnce(TestingContext)) {
    // We don't actually care if it fails
    let _ = env_logger::try_init();

    let backend_bits = util::backend_bits_from_env().unwrap_or(BackendBit::all());
    let instance = Instance::new(backend_bits);
    let adapter = pollster::block_on(util::initialize_adapter_from_env_or_default(
        &instance,
        backend_bits,
    ))
    .expect("could not find sutable adapter on the system");

    let adapter_info = adapter.get_info();
    let adapter_lowercase_name = adapter_info.name.to_lowercase();
    let adapter_features = adapter.features();
    let adapter_limits = adapter.limits();
    let adapter_downlevel_properties = adapter.get_downlevel_properties();

    let missing_features = parameters.required_features - adapter_features;
    if !missing_features.is_empty() {
        println!("TEST SKIPPED: MISSING FEATURES {:?}", missing_features);
        return;
    }

    if adapter_limits < parameters.required_limits {
        println!("TEST SKIPPED: LIMIT TOO LOW");
        return;
    }

    let missing_downlevel_flags =
        parameters.required_downlevel_properties.flags - adapter_downlevel_properties.flags;
    if !missing_downlevel_flags.is_empty() {
        println!(
            "TEST SKIPPED: MISSING DOWNLEVEL FLAGS {:?}",
            missing_downlevel_flags
        );
        return;
    }

    if adapter_downlevel_properties.shader_model
        < parameters.required_downlevel_properties.shader_model
    {
        println!(
            "TEST SKIPPED: LOW SHADER MODEL {:?}",
            adapter_downlevel_properties.shader_model
        );
        return;
    }

    let (device, queue) = pollster::block_on(initialize_device(
        &adapter,
        parameters.required_features,
        parameters.required_limits,
    ));

    let context = TestingContext {
        adapter,
        adapter_info: adapter_info.clone(),
        device,
        queue,
    };

    let panicked = catch_unwind(AssertUnwindSafe(|| test_function(context))).is_err();

    let failure_reason = parameters.failures.iter().find_map(
        |(backend_failure, vendor_failure, adapter_failure)| {
            let always =
                backend_failure.is_none() && vendor_failure.is_none() && adapter_failure.is_none();

            let expect_failure_backend = backend_failure
                .map(|f| f.contains(wgpu::BackendBit::from(adapter_info.backend)))
                .unwrap_or(true);
            let expect_failure_vendor = vendor_failure
                .map(|v| v == adapter_info.vendor)
                .unwrap_or(true);
            let expect_failure_adapter = adapter_failure
                .as_deref()
                .map(|f| adapter_lowercase_name.contains(f))
                .unwrap_or(true);

            if expect_failure_backend && expect_failure_vendor && expect_failure_adapter {
                if always {
                    Some(FailureReason::ALWAYS)
                } else {
                    let mut reason = FailureReason::empty();
                    reason.set(FailureReason::BACKEND, expect_failure_backend);
                    reason.set(FailureReason::VENDOR, expect_failure_vendor);
                    reason.set(FailureReason::ADAPTER, expect_failure_adapter);
                    Some(reason)
                }
            } else {
                None
            }
        },
    );

    let expect_failure = failure_reason.is_some();

    if panicked == expect_failure {
        // We got the conditions we expected
        if let Some(reason) = failure_reason {
            // Print out reason for the failure
            println!("GOT EXPECTED TEST FAILURE: {:?}", reason);
        }
    } else {
        if let Some(reason) = failure_reason {
            // We expected to fail, but things passed
            panic!("UNEXPECTED TEST PASS: {:?}", reason);
        } else {
            panic!("UNEXPECTED TEST FAILURE")
        }
    }
}
