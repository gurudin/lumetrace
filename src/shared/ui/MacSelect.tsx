import { Check, ChevronDown } from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { usePresence } from "./usePresence";
import "./mac-select.css";

export interface MacSelectOption<Value extends string> {
  value: Value;
  label: string;
  disabled?: boolean;
}

interface MacSelectProps<Value extends string> {
  value: Value;
  options: readonly MacSelectOption<Value>[];
  onChange: (value: Value) => void;
  ariaLabel: string;
  icon?: ReactNode;
  className?: string;
  disabled?: boolean;
  menuAlign?: "start" | "end";
  menuMinWidth?: number;
}

interface MenuPosition {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
}

const viewportMargin = 8;
const menuGap = 6;

function enabledIndex<Value extends string>(
  options: readonly MacSelectOption<Value>[],
  start: number,
  direction: 1 | -1,
) {
  if (options.length === 0) return -1;
  for (let offset = 0; offset < options.length; offset += 1) {
    const index = (start + offset * direction + options.length) % options.length;
    if (!options[index]?.disabled) return index;
  }
  return -1;
}

function isFocusableTarget(target: EventTarget | null) {
  return target instanceof Element && Boolean(target.closest(
    "button, a[href], input, select, textarea, [tabindex]:not([tabindex='-1'])",
  ));
}

function focusAdjacentElement(trigger: HTMLElement | null, backwards: boolean) {
  if (!trigger) return;
  const focusableElements = Array.from(document.querySelectorAll<HTMLElement>(
    "button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])",
  )).filter((element) => element.getClientRects().length > 0 && element.getAttribute("aria-hidden") !== "true");
  const triggerIndex = focusableElements.indexOf(trigger);
  const adjacentIndex = triggerIndex + (backwards ? -1 : 1);
  const adjacent = focusableElements[adjacentIndex]
    ?? focusableElements[backwards ? focusableElements.length - 1 : 0];
  (adjacent ?? trigger).focus();
}

export function MacSelect<Value extends string>({
  value,
  options,
  onChange,
  ariaLabel,
  icon,
  className,
  disabled = false,
  menuAlign = "start",
  menuMinWidth = 176,
}: MacSelectProps<Value>) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const typeaheadRef = useRef({ value: "", timer: 0 });
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [keyboardActive, setKeyboardActive] = useState(false);
  const [menuPosition, setMenuPosition] = useState<MenuPosition | null>(null);
  const presence = usePresence(open, 120);
  const listboxId = useId();
  const selectedIndex = options.findIndex((option) => option.value === value);
  const selectedOption = options[selectedIndex] ?? options.find((option) => !option.disabled);

  const updateMenuPosition = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const triggerRect = trigger.getBoundingClientRect();
    const measuredMenu = menuRef.current?.getBoundingClientRect();
    const width = Math.min(
      Math.max(triggerRect.width, menuMinWidth, measuredMenu?.width ?? 0),
      Math.max(0, window.innerWidth - viewportMargin * 2),
    );
    const estimatedHeight = Math.min(options.length * 32 + 12, 320);
    const menuHeight = measuredMenu?.height || estimatedHeight;
    const maxHeight = Math.max(96, window.innerHeight - viewportMargin * 2);
    let left = menuAlign === "end" ? triggerRect.right - width : triggerRect.left;
    left = Math.min(
      Math.max(viewportMargin, left),
      Math.max(viewportMargin, window.innerWidth - width - viewportMargin),
    );
    let top = triggerRect.bottom + menuGap;
    const availableBelow = window.innerHeight - top - viewportMargin;
    const availableAbove = triggerRect.top - menuGap - viewportMargin;
    if (menuHeight > availableBelow && availableAbove > availableBelow) {
      top = Math.max(viewportMargin, triggerRect.top - menuGap - Math.min(menuHeight, availableAbove));
    }
    setMenuPosition({
      top,
      left,
      width,
      maxHeight: Math.min(maxHeight, Math.max(96, Math.max(availableBelow, availableAbove))),
    });
  }, [menuAlign, menuMinWidth, options.length]);

  const closeMenu = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    setKeyboardActive(false);
    if (restoreFocus) window.requestAnimationFrame(() => triggerRef.current?.focus());
  }, []);

  const openMenu = useCallback((fromKeyboard: boolean, requestedIndex?: number) => {
    if (disabled || options.length === 0) return;
    const fallbackIndex = selectedIndex >= 0 && !options[selectedIndex]?.disabled
      ? selectedIndex
      : enabledIndex(options, 0, 1);
    setActiveIndex(requestedIndex ?? fallbackIndex);
    setKeyboardActive(fromKeyboard);
    updateMenuPosition();
    setOpen(true);
  }, [disabled, options, selectedIndex, updateMenuPosition]);

  const selectIndex = useCallback((index: number) => {
    const option = options[index];
    if (!option || option.disabled) return;
    if (option.value !== value) onChange(option.value);
    closeMenu(true);
  }, [closeMenu, onChange, options, value]);

  const moveActive = useCallback((direction: 1 | -1) => {
    const start = activeIndex < 0
      ? (direction === 1 ? 0 : options.length - 1)
      : (activeIndex + direction + options.length) % options.length;
    const nextIndex = enabledIndex(options, start, direction);
    if (nextIndex >= 0) setActiveIndex(nextIndex);
  }, [activeIndex, options]);

  const handleTypeahead = useCallback((key: string) => {
    if (key.length !== 1 || key.trim() === "") return -1;
    if (typeaheadRef.current.timer) window.clearTimeout(typeaheadRef.current.timer);
    const search = `${typeaheadRef.current.value}${key}`.toLocaleLowerCase();
    typeaheadRef.current.value = search;
    typeaheadRef.current.timer = window.setTimeout(() => {
      typeaheadRef.current.value = "";
      typeaheadRef.current.timer = 0;
    }, 600);
    const start = activeIndex < 0 ? 0 : activeIndex + 1;
    for (let offset = 0; offset < options.length; offset += 1) {
      const index = (start + offset) % options.length;
      const option = options[index];
      if (!option?.disabled && option.label.toLocaleLowerCase().startsWith(search)) {
        return index;
      }
    }
    return -1;
  }, [activeIndex, options]);

  const handleTriggerKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      openMenu(true);
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openMenu(true);
      return;
    }
    const matchingIndex = handleTypeahead(event.key);
    if (matchingIndex >= 0) {
      event.preventDefault();
      openMenu(true, matchingIndex);
    }
  };

  const handleMenuKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setKeyboardActive(true);
      moveActive(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      setKeyboardActive(true);
      setActiveIndex(enabledIndex(options, event.key === "Home" ? 0 : options.length - 1, event.key === "Home" ? 1 : -1));
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      selectIndex(activeIndex);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeMenu(true);
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      closeMenu(false);
      window.requestAnimationFrame(() => focusAdjacentElement(triggerRef.current, event.shiftKey));
      return;
    }
    const matchingIndex = handleTypeahead(event.key);
    if (matchingIndex >= 0) {
      event.preventDefault();
      setKeyboardActive(true);
      setActiveIndex(matchingIndex);
    }
  };

  useLayoutEffect(() => {
    if (!presence.mounted) return undefined;
    updateMenuPosition();
    const frame = window.requestAnimationFrame(updateMenuPosition);
    window.addEventListener("resize", updateMenuPosition);
    window.addEventListener("scroll", updateMenuPosition, true);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", updateMenuPosition);
      window.removeEventListener("scroll", updateMenuPosition, true);
    };
  }, [presence.mounted, updateMenuPosition]);

  useEffect(() => {
    if (!open || !presence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => menuRef.current?.focus());
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (triggerRef.current?.contains(target as Node) || menuRef.current?.contains(target as Node)) return;
      closeMenu(false);
      if (!isFocusableTarget(target)) {
        window.requestAnimationFrame(() => triggerRef.current?.focus());
      }
    };
    const handleWindowBlur = () => closeMenu(false);
    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("blur", handleWindowBlur);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("blur", handleWindowBlur);
    };
  }, [closeMenu, open, presence.mounted]);

  useEffect(() => () => {
    if (typeaheadRef.current.timer) window.clearTimeout(typeaheadRef.current.timer);
  }, []);

  const rootClassName = `mac-select${open ? " is-open" : ""}${className ? ` ${className}` : ""}`;
  const menuStyle = menuPosition ? ({
    top: `${menuPosition.top}px`,
    left: `${menuPosition.left}px`,
    width: `${menuPosition.width}px`,
    maxHeight: `${menuPosition.maxHeight}px`,
  } satisfies CSSProperties) : undefined;

  return (
    <div className={rootClassName}>
      <button
        ref={triggerRef}
        className="mac-select-trigger"
        type="button"
        disabled={disabled}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        onClick={() => open ? closeMenu(false) : openMenu(false)}
        onKeyDown={handleTriggerKeyDown}
      >
        {icon ? <span className="mac-select-trigger-icon" aria-hidden="true">{icon}</span> : null}
        <span className="mac-select-trigger-label">{selectedOption?.label ?? ""}</span>
        <ChevronDown className="mac-select-trigger-chevron" size={13} aria-hidden="true" />
      </button>

      {presence.mounted && menuPosition ? createPortal(
        <div
          ref={menuRef}
          id={listboxId}
          className="mac-select-menu"
          style={menuStyle}
          role="listbox"
          tabIndex={-1}
          aria-label={ariaLabel}
          aria-activedescendant={activeIndex >= 0 ? `${listboxId}-option-${activeIndex}` : undefined}
          data-state={presence.state}
          data-keyboard-active={keyboardActive ? "true" : "false"}
          onKeyDown={handleMenuKeyDown}
        >
          {options.map((option, index) => {
            const selected = option.value === value;
            return (
              <div
                key={option.value}
                id={`${listboxId}-option-${index}`}
                className={`mac-select-option${selected ? " is-selected" : ""}${activeIndex === index ? " is-active" : ""}`}
                role="option"
                aria-selected={selected}
                aria-disabled={option.disabled || undefined}
                onPointerMove={() => {
                  if (option.disabled) return;
                  setKeyboardActive(false);
                  setActiveIndex(index);
                }}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => selectIndex(index)}
              >
                <span className="mac-select-option-check" aria-hidden="true">
                  {selected ? <Check size={15} strokeWidth={2.2} /> : null}
                </span>
                <span>{option.label}</span>
              </div>
            );
          })}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}
