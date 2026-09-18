impl SemanticMarkerScanner {
    fn scan(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.in_string {
                self.scan_string_byte(byte);
            } else {
                self.scan_non_string_byte(byte);
            }
        }
    }

    fn scan_string_byte(&mut self, byte: u8) {
        self.track_selected_value_size();
        if self.scan_unicode_escape_byte(byte) || self.scan_escaped_byte(byte) {
            return;
        }
        match byte {
            b'\\' => self.escaped = true,
            b'"' => self.finish_string(),
            _ => self.push_string_byte(byte),
        }
    }

    fn track_selected_value_size(&mut self) {
        self.raw_string_bytes = self.raw_string_bytes.saturating_add(1);
        let selected_value = self.string_is_value
            && self.value_key.is_some_and(|(key, len)| {
                matches!(
                    &key[..len],
                    b"model"
                        | b"sticky_key"
                        | b"stickyKey"
                        | b"prompt_cache_key"
                        | b"promptCacheKey"
                        | b"service_tier"
                        | b"serviceTier"
                        | b"reasoning_effort"
                        | b"effort"
                        | b"type"
                )
            });
        if selected_value && self.raw_string_bytes > REQUEST_SEMANTIC_STRING_MAX_BYTES {
            self.semantic_limit_exceeded = true;
        }
    }

    fn scan_unicode_escape_byte(&mut self, byte: u8) -> bool {
        if self.unicode_remaining == 0 {
            return false;
        }
        let Some(digit) = (byte as char).to_digit(16) else {
            self.string_overflow = true;
            self.unicode_remaining = 0;
            return true;
        };
        self.unicode_value = (self.unicode_value << 4) | digit as u16;
        self.unicode_remaining -= 1;
        if self.unicode_remaining == 0 {
            if self.unicode_value <= 0x7f {
                self.push_string_byte(self.unicode_value as u8);
            } else {
                self.string_overflow = true;
            }
        }
        true
    }

    fn scan_escaped_byte(&mut self, byte: u8) -> bool {
        if !self.escaped {
            return false;
        }
        self.escaped = false;
        if byte == b'u' {
            self.unicode_remaining = 4;
            self.unicode_value = 0;
            return true;
        }
        let decoded = match byte {
            b'"' | b'\\' | b'/' => byte,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            _ => {
                self.string_overflow = true;
                return true;
            }
        };
        self.push_string_byte(decoded);
        true
    }

    fn push_string_byte(&mut self, byte: u8) {
        if self.string_len < self.string.len() {
            self.string[self.string_len] = byte;
            self.string_len += 1;
        } else {
            self.string_overflow = true;
        }
    }

    fn finish_string(&mut self) {
        self.in_string = false;
        if self.string_overflow {
            if self.string_is_value {
                self.value_key = None;
            }
            return;
        }
        let captured = (self.string, self.string_len);
        if self.string_is_value {
            self.record_tool_type_marker(&captured);
            self.value_key = None;
        } else {
            self.candidate = Some(captured);
        }
    }

    fn record_tool_type_marker(&mut self, captured: &([u8; 32], usize)) {
        if !self
            .value_key
            .is_some_and(|(key, len)| &key[..len] == b"type")
        {
            return;
        }
        let value = &captured.0[..captured.1];
        self.contains_encrypted_content |= value == b"encrypted_content";
        self.contains_image_generation |= value == b"image_generation";
        self.contains_codex_image_generation |= value == b"image_gen.imagegen";
    }

    fn scan_non_string_byte(&mut self, byte: u8) {
        if byte == b'"' {
            self.begin_string();
        } else if !byte.is_ascii_whitespace() {
            self.scan_structural_byte(byte);
        }
    }

    fn begin_string(&mut self) {
        self.in_string = true;
        self.escaped = false;
        self.unicode_remaining = 0;
        self.unicode_value = 0;
        self.string_len = 0;
        self.raw_string_bytes = 0;
        self.string_overflow = false;
        self.string_is_value = self.value_key.is_some();
    }

    fn scan_structural_byte(&mut self, byte: u8) {
        if byte == b':' {
            self.value_key = self.candidate.take();
            if self
                .value_key
                .is_some_and(|(key, len)| &key[..len] == b"encrypted_content")
            {
                self.contains_encrypted_content = true;
            }
        } else if self.value_key.is_some() {
            self.value_key = None;
        }
    }
}
