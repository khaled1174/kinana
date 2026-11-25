use super::{
    AllowedValues, classification_data, proposal_schema, render_prompt, schema_dimension_bytes,
};
use crate::{Error, Result, SkillRecord, TokenCalibration, TokenEstimate};

pub(crate) fn skill_tokens(id: &str, text: &str, files: &[String]) -> u64 {
    let id_bytes = serde_json::to_string(id).expect("serialize ID").len() - 2;
    let file_bytes = serde_json::to_string(files)
        .expect("serialize file names")
        .len()
        - 2;
    (text.len() as u64 + id_bytes as u64 + file_bytes as u64).div_ceil(4)
}

pub(crate) fn estimate(
    skills: &[(&str, &str, &[String])],
    allowed: &AllowedValues<'_>,
    existing: &[SkillRecord],
    provider_calls: usize,
    guide: Option<&str>,
    uncalibrated: bool,
) -> Result<TokenEstimate> {
    if provider_calls == 0 {
        return Ok(TokenEstimate::default());
    }
    let calibration: TokenCalibration = serde_json::from_str(include_str!("calibration.json"))
        .map_err(|error| Error::new("INVALID_TOKEN_CALIBRATION", error.to_string()))?;
    let measured = calibration.input_tokens
        + calibration.cache_creation_input_tokens
        + calibration.cache_read_input_tokens;
    if measured != calibration.total_input_tokens
        || calibration.prompt_utf8_bytes.div_ceil(4) != calibration.estimated_prompt_tokens
        || calibration.cli_overhead_tokens + calibration.estimated_prompt_tokens != measured
    {
        return Err(Error::new(
            "INVALID_TOKEN_CALIBRATION",
            "Token calibration must agree with measured adapter usage and prompt bytes",
        ));
    }
    // Common prompt data is fixed for this batch. CLI overhead is calibrated
    // from the actual isolated adapter, including cache-read input tokens.
    let data = classification_data("", &[], allowed, &[]);
    let prompt = render_prompt(
        "",
        &data,
        &proposal_schema(allowed).to_string(),
        "skill-00000000000000000000000000000000",
        guide,
    );
    // The rendered prompt already contains the dimension guidance, both lists,
    // the guide and the eleven-field schema. The second schema copy travels as
    // the `--json-schema` tool definition inside the calibrated CLI overhead,
    // which was measured with the eight-field schema; add what the three
    // fields added since (stack, stage, category_reason) add.
    // Pi (0.3) has no calibration yet: the estimate is the prompt alone, with
    // no CLI overhead, and says so by carrying no calibration.
    let base_per_call = if uncalibrated {
        (prompt.len() as u64).div_ceil(4)
    } else {
        calibration.cli_overhead_tokens
            + (prompt.len() as u64).div_ceil(4)
            + schema_dimension_bytes(allowed).div_ceil(4)
    };
    let fixed_tokens_total = skills
        .iter()
        .cycle()
        .take(provider_calls)
        .map(|(id, _, _)| {
            let inventory = super::overlap_inventory(id, existing);
            let inventory_bytes = serde_json::to_vec(&inventory)
                .expect("serialize overlap inventory")
                .len()
                - 2;
            base_per_call + ((prompt.len() + inventory_bytes) as u64).div_ceil(4)
                - (prompt.len() as u64).div_ceil(4)
        })
        .sum::<u64>();
    let fixed_tokens_per_call = fixed_tokens_total.div_ceil(provider_calls as u64);
    let skill_tokens_total = skills
        .iter()
        .map(|(id, text, files)| skill_tokens(id, text, files))
        .sum::<u64>();
    let total_input_tokens = fixed_tokens_total.saturating_add(skill_tokens_total);
    Ok(TokenEstimate {
        fixed_tokens_per_call,
        fixed_tokens_total,
        skill_tokens_total,
        total_input_tokens,
        retry_limit_input_tokens: total_input_tokens.saturating_mul(2),
        calibration: (!uncalibrated).then_some(calibration),
        claude: Default::default(),
    })
}
