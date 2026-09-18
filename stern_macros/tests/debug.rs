use slint::private_unstable_api::re_exports as sp;

#[stern::adopter]
#[derive(sp :: FieldOffsets, Default)]
#[const_field_offset(sp::const_field_offset)]
#[repr(C)]
#[pin]
pub struct InnerScoreAdopter {
    r#score: sp::Property<i32>,
    globals: sp::OnceCell<sp::Weak<SharedGlobals>>,
}

#[allow(unused)]
pub struct r#ScoreAdopter<'a>(
    ::core::pin::Pin<sp::Rc<InnerScoreAdopter>>,
    ::core::marker::PhantomData<&'a InnerScoreAdopter>,
);

impl<'a> r#ScoreAdopter<'a> {
    #[allow(dead_code)]
    pub fn get_score(&self) -> i32 {
        #[allow(unused_imports)]
        let _self = self.0.as_ref();
        { *&InnerScoreAdopter::FIELD_OFFSETS.r#score() }
            .apply_pin(_self)
            .get()
    }
    #[allow(dead_code)]
    pub fn set_score(&self, value: i32) {
        #[allow(unused_imports)]
        let _self = self.0.as_ref();
        { *&InnerScoreAdopter::FIELD_OFFSETS.r#score() }
            .apply_pin(_self)
            .set(value as _)
    }
}

impl slint::StrongHandle for r#ScoreAdopter<'static> {
    type WeakInner = sp::Weak<InnerScoreAdopter>;
    fn upgrade_from_weak_inner(inner: &Self::WeakInner) -> ::core::option::Option<Self> {
        let inner = ::core::pin::Pin::new(inner.upgrade()?);
        ::core::option::Option::Some(Self(inner, ::core::marker::PhantomData::default()))
    }
}

#[allow(non_snake_case)]
struct SharedGlobals {
    global_ScoreAdopter: ::core::pin::Pin<sp::Rc<InnerScoreAdopter>>,
    window_adapter: sp::OnceCell<sp::WindowAdapterRc>,
    root_item_tree_weak: sp::VWeak<sp::ItemTreeVTable>,
}

#[test]
fn test() {
    // define_score_mapper!();
}
