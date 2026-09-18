/*use std::cell::Cell;

use slint::private_unstable_api::re_exports as sp;
use stern as fw;
use stern_macros::adopter;

#[adopter]
struct InnerTestAdopter {
    count: sp::Property<i32>,
    increment: sp::Callback<(), ()>,
    callback_tracker_increment: sp::Property<()>,
    globals: sp::OnceCell<sp::Weak<()>>, // 本来は()ではなくSharedGlobals
}

struct Adopter(InnerTestAdopter);*/
