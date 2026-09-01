//! The stable identity of this provider.
//!
//! A provider carries a stable id. The CLI names it, the session file records it, and a
//! resumed session identifies its provider by this string. Changing it is a wire break, so
//! this test pins the exact value.

use rho_core::{Provider, Secret};
use rho_provider_anthropic::{AnthropicConfig, AnthropicProvider};

#[test]
fn the_provider_id_is_anthropic() {
    let provider =
        AnthropicProvider::new(AnthropicConfig::against_anthropic(Secret::from("any-key")));
    assert_eq!(provider.id(), "anthropic");
}
