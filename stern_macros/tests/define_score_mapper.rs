trait ScorePropertyMappers {
    type ScoreMappedType;
    fn map_score(value: Self::ScoreMappedType) -> i32;
}

macro_rules! define_score_mapper {
    {$(score to $score_typ:ty { $score_mapper:expr },)?} => {

    };
}

stern::define_mapper_impl! {
    base_name : Score,
    properties : {
        score : {
            slint_typ : i32,
        },
    }
}

#[test]
fn use_mapper() {
    let m = Mapper;
}
