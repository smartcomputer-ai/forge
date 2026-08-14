use sha2::{Digest as _, Sha256};

use environment_protocol::control::targets::ProviderBindingContext;

pub const META_PREFIX: &str = "user.lightspeed.";

pub fn stable_component(kind: &str, parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hex(&hash.finalize()[..10])
}

pub fn project_name(binding: &ProviderBindingContext) -> String {
    format!(
        "ls-{}",
        stable_component("binding", &[&binding.universe_id, &binding.binding_id])
    )
}

pub fn network_name(binding: &ProviderBindingContext) -> String {
    let component = stable_component("binding", &[&binding.universe_id, &binding.binding_id]);
    // A managed bridge's Incus network name also becomes its Linux interface
    // name, whose hard limit is 15 bytes. Keep 48 bits of the scoped digest
    // while making the resource kind visible within that limit.
    format!("ls{}n", &component[..12])
}
pub fn profile_name(binding: &ProviderBindingContext) -> String {
    format!("{}-vm", project_name(binding))
}
pub fn acl_name(binding: &ProviderBindingContext) -> String {
    format!("{}-acl", project_name(binding))
}

pub fn instance_name(
    universe_id: &str,
    binding_id: &str,
    environment_id: &str,
    incarnation_id: &str,
) -> String {
    format!(
        "ls-{}",
        stable_component(
            "instance",
            &[universe_id, binding_id, environment_id, incarnation_id]
        )
    )
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 15) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_stable_and_scoped() {
        let a = instance_name("u", "b", "e", "i");
        assert_eq!(a, instance_name("u", "b", "e", "i"));
        assert_ne!(a, instance_name("u2", "b", "e", "i"));
    }

    #[test]
    fn binding_network_names_fit_the_linux_interface_limit() {
        let binding = ProviderBindingContext {
            universe_id: "0d3d9e5e-2428-4e60-b66e-8d4520f64e5d".to_owned(),
            binding_id: "hz02-incus".to_owned(),
        };
        let other = ProviderBindingContext {
            universe_id: "61092591-cd39-4504-b845-a817e3d9cb71".to_owned(),
            binding_id: "hz02-incus".to_owned(),
        };

        assert_eq!(network_name(&binding).len(), 15);
        assert_ne!(network_name(&binding), network_name(&other));
    }
}
