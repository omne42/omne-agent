fn catalog_structured_error(code: &str) -> anyhow::Result<structured_text_protocol::StructuredTextData> {
    catalog_structured_error_with(code, |_| Ok(()))
}

fn catalog_structured_error_with<F>(
    code: &str,
    build: F,
) -> anyhow::Result<structured_text_protocol::StructuredTextData>
where
    F: FnOnce(
        &mut structured_text_kit::CatalogText,
    ) -> Result<(), structured_text_kit::StructuredTextValidationError>,
{
    let mut message =
        structured_text_kit::CatalogText::try_new(code.to_owned()).map_err(anyhow::Error::new)?;
    build(&mut message).map_err(anyhow::Error::new)?;
    Ok(structured_text_protocol::StructuredTextData::from(
        &structured_text_kit::StructuredText::from(message),
    ))
}

fn structured_error_code(
    structured_error: &structured_text_protocol::StructuredTextData,
) -> Option<String> {
    structured_error.catalog_code().map(ToOwned::to_owned)
}

fn structured_error_from_result_value(
    value: &Value,
) -> Option<structured_text_protocol::StructuredTextData> {
    value
        .get("structured_error")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}
