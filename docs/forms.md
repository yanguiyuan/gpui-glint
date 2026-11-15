# Controlled forms

`Form`, `FormSection`, `FormField` and `FormActions` compose existing GPUI controls. They provide layout, theme-aware focus/error presentation and submit intents. Field values, validation, authentication and persistence remain host responsibilities; there is no schema or field registry.

## Composition

```rust,ignore
let field = FormField::new("username", "用户名", self.username.clone())
    .theme(theme)
    .focused(self.username.read(cx).focus_handle(cx).is_focused(window))
    .required(true)
    .description("使用你的账号名称")
    .render();

let entries = [self.username.clone(), self.password.clone()];
let mut form = Form::new()
    .id("sign-in")
    .theme(theme)
    .busy(self.busy)
    .submit_on_enter(true)
    .submit_guard(move |_, app| entries.iter().all(|entry| !entry.read(app).is_composing()))
    .on_submit(cx.listener(|this, _: &FormSubmit, window, cx| this.submit(window, cx)))
    .child(FormSection::new().theme(theme).child(field).render())
    .actions(FormActions::new("登录").theme(theme).busy_label("正在登录…"));
if let Some(error) = &self.error { form = form.error(error.clone()); }
form.render()
```

## Components

- `FormField`: label, optional required marker, themed control frame, focus ring, optional trailing element, description and field error. Errors replace descriptions. The supplied control is a direct child of the fixed-height row, so controls such as `TextEntry` retain their percentage-height hit area. Use `control_height` for taller controls; the host supplies focus state and observes its controls to trigger updates.
- `FormSection`: optional heading/description and vertically spaced fields. Implements `ParentElement` for `.child` / `.children` composition.
- `FormActions`: a primary submit button and optional host-supplied secondary control. It also works standalone with its own `on_submit`; busy and disabled actions cannot invoke that handler. Primary action sizing and subdued shadow match the Glint Frame login design.
- `Form`: sections, form-level feedback and action placement. Errors take precedence over hints; hints are hidden while busy. Standard `FormActions` inherit busy/disabled state and, unless overridden, the form's submit handler. `actions_element` accepts custom actions whose state and events remain host-owned.

## Submission and input behavior

`submit_on_enter` is off by default. Enable it only for single-line forms. Plain Enter is consumed when enabled (also while busy/disabled) so it does not insert a newline into a single-line control; modified Enter is left untouched. `submit_guard` can return false while an IME is composing or a focused control owns Enter. In that case the event is not consumed. Keep business validation in the host's submit callback.

`busy` and `disabled` suppress form submission and disable standard actions; they do not disable arbitrary supplied input widgets or custom secondary actions. Set the controls' own enabled state in the host when required. `required` is a visual marker and does not validate values.

Give multiple forms in the same view distinct `id`s. Field ids identify the input row; field description/error debug selectors append `-description` / `-error`. Form feedback and action ids can be customized with `error_id`, `hint_id` and `FormActions::id`.

The playground's **表单** page demonstrates a connection-settings form using local host-side validation, with no network requests.
