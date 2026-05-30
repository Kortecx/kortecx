//! Ergonomic conversion from the data-plane's `kx_dataset::ContentSchema` to the
//! parser-free [`SchemaTag`] a `SchemaSpec` validates against. Lets an author
//! holding a `TypedRef` derive a schema critic without restating the tag.
//!
//! A free function (not a `From` impl): both `SchemaTag` and `ContentSchema` are
//! foreign to this crate, so the orphan rule forbids the trait impl.
//!
//! [`SchemaTag`]: kx_critic_types::SchemaTag

use kx_critic_types::{SchemaTag, TensorDTypeTag};
use kx_dataset::{ContentSchema, TensorDType};

/// Convert a `kx_dataset::ContentSchema` into the parser-free [`SchemaTag`].
#[must_use]
pub fn schema_tag_of(schema: &ContentSchema) -> SchemaTag {
    match schema {
        ContentSchema::Blob => SchemaTag::Blob,
        ContentSchema::Text => SchemaTag::Text,
        ContentSchema::Json => SchemaTag::Json,
        ContentSchema::Tensor { dtype, shape } => SchemaTag::Tensor {
            dtype: dtype_tag(*dtype),
            shape: shape.clone(),
        },
        ContentSchema::Vector { dim } => SchemaTag::Vector { dim: *dim },
        ContentSchema::Image => SchemaTag::Image,
        ContentSchema::Audio => SchemaTag::Audio,
    }
}

const fn dtype_tag(dtype: TensorDType) -> TensorDTypeTag {
    match dtype {
        TensorDType::F32 => TensorDTypeTag::F32,
        TensorDType::F16 => TensorDTypeTag::F16,
        TensorDType::BF16 => TensorDTypeTag::BF16,
        TensorDType::I64 => TensorDTypeTag::I64,
        TensorDType::I32 => TensorDTypeTag::I32,
        TensorDType::U8 => TensorDTypeTag::U8,
        TensorDType::Bool => TensorDTypeTag::Bool,
    }
}
