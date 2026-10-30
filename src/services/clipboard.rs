use std::fmt;

/// OS clipboard with an in-process fallback when no system clipboard is available
/// (e.g. headless sessions), so copy/paste still works within the editor.
pub struct Clipboard {
    system: Option<arboard::Clipboard>,
    fallback: String,
}

impl Clipboard {
    pub fn system() -> Self {
        Self {
            system: arboard::Clipboard::new().ok(),
            fallback: String::new(),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            system: None,
            fallback: String::new(),
        }
    }

    pub fn set_text(&mut self, text: String) -> Result<(), String> {
        match &mut self.system {
            Some(system) => system
                .set_text(text)
                .map_err(|err| format!("Clipboard error: {err}")),
            None => {
                self.fallback = text;
                Ok(())
            }
        }
    }

    pub fn get_text(&mut self) -> Result<String, String> {
        match &mut self.system {
            Some(system) => match system.get_text() {
                Ok(text) => Ok(text),
                Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
                Err(err) => Err(format!("Clipboard error: {err}")),
            },
            None => Ok(self.fallback.clone()),
        }
    }
}

impl fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Clipboard")
            .field("system", &self.system.is_some())
            .finish_non_exhaustive()
    }
}
