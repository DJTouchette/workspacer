import { describe, it, expect } from 'vitest';
import { productionFiles } from '../../../tests/support/rustProductionSource';
describe('Rust production module ancestry', () => {
  it('propagates test-only ownership through ordinary and explicit-path descendants', () => {
    const files = new Map([
      ['src/lib.rs', '#[cfg(test)] #[path="audit.rs"] mod audit;'],
      ['src/audit.rs', '#[path="broker_tests.rs"] mod child;'],
      ['src/broker_tests.rs', 'mod grandchild;'],
      ['src/broker_tests/grandchild.rs', 'fn test(){ Event {data:None}; }'],
      ['src/unreferenced_tests.rs', 'fn production(){ Event {data:None}; }'],
    ]);
    expect([...productionFiles(files).keys()]).toEqual(['src/lib.rs', 'src/unreferenced_tests.rs']);
    files.set(
      'src/lib.rs',
      '#[cfg(test)] #[path="audit.rs"] mod audit; #[path="broker_tests.rs"] mod ordinary;',
    );
    const live = productionFiles(files);
    expect(live.has('src/audit.rs')).toBe(false);
    expect(live.has('src/broker_tests.rs')).toBe(true);
    expect(live.has('src/broker_tests/grandchild.rs')).toBe(true);
  });
  it('follows inline scopes and cfg conjunctions without excluding possible production alternatives', () => {
    const files = new Map([
      [
        'src/lib.rs',
        '#[cfg(all(test,unix))] mod tests { #[path="nested.rs"] mod child; fn local(){Event {data:None};}}',
      ],
      ['src/tests/nested.rs', 'fn child(){Event {data:None};}'],
    ]);
    const onlyTests = productionFiles(files);
    expect(onlyTests.has('src/tests/nested.rs')).toBe(false);
    expect(onlyTests.get('src/lib.rs')).not.toContain('Event');
    files.set('src/lib.rs', files.get('src/lib.rs')!.replace('all(test,unix)', 'any(test,unix)'));
    expect(productionFiles(files).has('src/tests/nested.rs')).toBe(true);
    expect(productionFiles(files).get('src/lib.rs')).toContain('Event');
  });
  it('uses declaration ancestry rather than a tests-like filename and handles attribute order/CRLF', () => {
    const files = new Map([
      ['src/lib.rs', '#[path="test_helpers.rs"]\r\n#[cfg(test)]\r\nmod examples;'],
      ['src/test_helpers.rs', 'mod child;'],
      ['src/test_helpers/child.rs', 'fn child(){}'],
    ]);
    expect([...productionFiles(files).keys()]).toEqual(['src/lib.rs']);
    files.set('src/lib.rs', '#[path="test_helpers.rs"] mod examples;');
    expect(productionFiles(files).size).toBe(3);
  });
});
