#[derive(Debug, Default)]
struct ReplaySnapshotRouteValue {
    value: Value,
    sticky_projection_valid: bool,
}

#[derive(Debug, Clone, Copy)]
enum ReplaySnapshotRouteValueScope {
    Root,
    Metadata,
    Nested,
}

impl ReplaySnapshotRouteValueScope {
    fn child(self, key: &str) -> Self {
        if matches!(self, Self::Root) && key == "metadata" {
            Self::Metadata
        } else {
            Self::Nested
        }
    }

    fn sticky_projection_key_bit(self, key: &str) -> Option<u8> {
        match self {
            Self::Root => match key {
                "metadata" => Some(1 << 0),
                "sticky_key" => Some(1 << 1),
                "stickyKey" => Some(1 << 2),
                "prompt_cache_key" => Some(1 << 3),
                "promptCacheKey" => Some(1 << 4),
                _ => None,
            },
            Self::Metadata => match key {
                "sticky_key" => Some(1 << 0),
                "stickyKey" => Some(1 << 1),
                "prompt_cache_key" => Some(1 << 2),
                "promptCacheKey" => Some(1 << 3),
                _ => None,
            },
            Self::Nested => None,
        }
    }
}

struct ReplaySnapshotRouteValueSeed<'a> {
    sticky_projection_valid: &'a mut bool,
    scope: ReplaySnapshotRouteValueScope,
}

struct ReplaySnapshotRouteValueVisitor<'a> {
    sticky_projection_valid: &'a mut bool,
    scope: ReplaySnapshotRouteValueScope,
}

impl<'de> DeserializeSeed<'de> for ReplaySnapshotRouteValueSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ReplaySnapshotRouteValueVisitor {
            sticky_projection_valid: self.sticky_projection_valid,
            scope: self.scope,
        })
    }
}

impl<'de> Visitor<'de> for ReplaySnapshotRouteValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(ReplaySnapshotRouteValueSeed {
            sticky_projection_valid: &mut *self.sticky_projection_valid,
            scope: ReplaySnapshotRouteValueScope::Nested,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        deserialize_replay_snapshot_route_value_map(
            &mut map,
            self.sticky_projection_valid,
            self.scope,
        )
    }
}

fn deserialize_replay_snapshot_route_value_map<'de, A>(
    map: &mut A,
    sticky_projection_valid: &mut bool,
    scope: ReplaySnapshotRouteValueScope,
) -> Result<Value, A::Error>
where
    A: MapAccess<'de>,
{
    let mut object = serde_json::Map::new();
    let mut seen_sticky_projection_key_bits = 0_u8;
    while let Some(key) = map.next_key::<String>()? {
        if let Some(bit) = scope.sticky_projection_key_bit(&key) {
            if seen_sticky_projection_key_bits & bit != 0 {
                *sticky_projection_valid = false;
            }
            seen_sticky_projection_key_bits |= bit;
        }
        let value = map.next_value_seed(ReplaySnapshotRouteValueSeed {
            sticky_projection_valid: &mut *sticky_projection_valid,
            scope: scope.child(&key),
        })?;
        object.insert(key, value);
    }
    Ok(Value::Object(object))
}

impl<'de> Deserialize<'de> for ReplaySnapshotRouteValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut sticky_projection_valid = true;
        let value = ReplaySnapshotRouteValueSeed {
            sticky_projection_valid: &mut sticky_projection_valid,
            scope: ReplaySnapshotRouteValueScope::Root,
        }
        .deserialize(deserializer)?;
        Ok(Self {
            value,
            sticky_projection_valid,
        })
    }
}
