//! Checks every shader variant the renderer builds without a GPU, so CI without an adapter still catches a broken one.

use naga::back::PipelineConstants;
use naga::back::pipeline_constants::process_overrides;
use naga::valid::{Capabilities, ValidationFlags, Validator};

#[test]
fn every_variant_validates() {
    let module = naga::front::wgsl::parse_str(include_str!("../src/shaders.wgsl")).unwrap();
    let info = Validator::new(ValidationFlags::all(), Capabilities::default()).validate(&module).unwrap();
    for fs in ["fs_mesh", "fs_lit"] {
        for layers in [1.0, 2.0] {
            for anti_tile in [0.0, 1.0, 2.0] {
                let constants = PipelineConstants::from_iter([("LAYERS".to_string(), layers), ("ANTI_TILE".to_string(), anti_tile)]);
                let result = process_overrides(&module, &info, Some((naga::ShaderStage::Fragment, fs)), &constants);
                assert!(result.is_ok(), "{fs} with LAYERS {layers} and ANTI_TILE {anti_tile}: {:?}", result.err());
            }
        }
    }
}
