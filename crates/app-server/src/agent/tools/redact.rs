fn redact_tool_output(mut value: Value) -> Value {
    fn walk(value: &mut Value) {
        match value {
            Value::String(s) => {
                *s = omne_core::redact_text(s);
            }
            Value::Array(items) => {
                for item in items {
                    walk(item);
                }
            }
            Value::Object(obj) => {
                for v in obj.values_mut() {
                    walk(v);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    value
}
