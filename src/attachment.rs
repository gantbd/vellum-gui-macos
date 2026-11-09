//! Bind framework-owned focus, drag and IME state to its original attachment.
//! Message validation alone cannot guard state the widget retains before it
//! emits a message. No engine identity, counter or composition owner is added.
use crate::ViewToken;
use iced::{
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, operation::Focusable, tree},
    },
};

struct State {
    token: ViewToken,
    focused: bool,
    resume_on_key: bool,
    window_focused: bool,
}
struct Focus {
    restore: bool,
    observed: bool,
}
impl Operation for Focus {
    fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn Operation)) {
        visit(self);
    }
    fn focusable(
        &mut self,
        _: Option<&iced::advanced::widget::Id>,
        _: Rectangle,
        state: &mut dyn Focusable,
    ) {
        if self.restore {
            state.focus();
        }
        self.observed |= state.is_focused();
    }
}
struct Attached<'a, Message> {
    token: ViewToken,
    child: Element<'a, Message>,
}

pub(crate) fn attached<'a, Message: 'a>(
    token: ViewToken,
    child: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    Element::new(Attached {
        token,
        child: child.into(),
    })
}

impl<Message> Widget<Message, Theme, Renderer> for Attached<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            token: self.token.clone(),
            focused: false,
            resume_on_key: false,
            window_focused: true,
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.child.as_widget())]
    }
    fn diff(&self, tree: &mut Tree) {
        let previous = tree.state.downcast_ref::<State>();
        if previous.token != self.token {
            // Retain the original Arc identity, not a recyclable pointer key.
            // Keyboard users need not click after Undo or F6. A subsequent
            // explicit key may resume prior editor focus; stale IME/drag events
            // cannot. An intervening pointer press or window blur cancels it.
            let resume_on_key =
                previous.window_focused && (previous.focused || previous.resume_on_key);
            let window_focused = previous.window_focused;
            tree.state = tree::State::new(State {
                token: self.token.clone(),
                focused: false,
                resume_on_key,
                window_focused,
            });
            tree.children = self.children();
        } else {
            tree.children[0].diff(self.child.as_widget());
        }
    }
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.child.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.child.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.child
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
        let mut focus = Focus {
            restore: false,
            observed: false,
        };
        self.child
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut focus);
        tree.state.downcast_mut::<State>().focused = focus.observed;
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if matches!(event, Event::Window(iced::window::Event::Unfocused)) {
            state.window_focused = false;
            // The pinned widget only suspends caret blinking on window blur.
            // Cancel its transient preedit through its public event contract;
            // preserve logical caret/selection and native source/history.
            self.child.as_widget_mut().update(
                &mut tree.children[0],
                &Event::InputMethod(iced::advanced::input_method::Event::Closed),
                layout,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        } else if matches!(event, Event::Window(iced::window::Event::Focused)) {
            state.window_focused = true;
        }
        if !state.window_focused && matches!(event, Event::InputMethod(_)) {
            return;
        }
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(_))
                | Event::Window(iced::window::Event::Unfocused)
        ) {
            state.resume_on_key = false;
        }
        if state.resume_on_key
            && !shell.is_event_captured()
            && matches!(
                event,
                Event::Keyboard(iced::keyboard::Event::KeyPressed { .. })
            )
        {
            self.child.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                &mut Focus {
                    restore: true,
                    observed: false,
                },
            );
            state.resume_on_key = false;
        }
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let mut focus = Focus {
            restore: false,
            observed: false,
        };
        self.child
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut focus);
        state.focused = focus.observed;
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.child.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.child.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
