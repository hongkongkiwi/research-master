use quick_xml::events::Event;
use quick_xml::reader::Reader;

pub(crate) enum XmlEvent<'a> {
    Text(Result<String, quick_xml::Error>),
    Event(Event<'a>),
}

pub(crate) struct XmlReader<'a> {
    reader: Reader<&'a [u8]>,
    pending: Option<Result<Event<'a>, quick_xml::Error>>,
}

impl<'a> XmlReader<'a> {
    pub(crate) fn new(xml: &'a str) -> Self {
        Self {
            reader: Reader::from_str(xml),
            pending: None,
        }
    }

    pub(crate) fn read_event(&mut self) -> Result<XmlEvent<'a>, quick_xml::Error> {
        let event = match self.pending.take() {
            Some(event) => event?,
            None => self.reader.read_event()?,
        };
        if !matches!(event, Event::Text(_) | Event::GeneralRef(_)) {
            return Ok(XmlEvent::Event(event));
        }

        let mut raw = String::new();
        let mut error = None;
        let mut event = event;
        loop {
            let decoded = match &event {
                Event::Text(text) => text.decode(),
                Event::GeneralRef(reference) => {
                    raw.push('&');
                    reference.decode()
                }
                _ => unreachable!(),
            };
            match decoded {
                Ok(text) => raw.push_str(&text),
                Err(err) => error = Some(err.into()),
            }
            if matches!(event, Event::GeneralRef(_)) {
                raw.push(';');
            }

            match self.reader.read_event() {
                Ok(next @ (Event::Text(_) | Event::GeneralRef(_))) => event = next,
                next => {
                    self.pending = Some(next);
                    break;
                }
            }
        }

        Ok(XmlEvent::Text(match error {
            Some(err) => Err(err),
            None => quick_xml::escape::unescape(&raw)
                .map(|text| text.into_owned())
                .map_err(Into::into),
        }))
    }
}
