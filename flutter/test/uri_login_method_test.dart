import 'dart:convert';

import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/generated_bridge.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:flutter_test/flutter_test.dart';

class _RendezvousBinding extends Fake implements RustdeskImpl {
  bool matches = false;
  bool rejectBare = false;
  String? selectedWebsocket;
  String configuredBareKey = '';
  final requests = <(String, String, bool)>[];

  @override
  Future<List<String>> mainResolveUriTarget(
      {required String target,
      required String key,
      required bool preferCurrent,
      String? recentEndpoint,
      dynamic hint}) async {
    requests.add((target, key, preferCurrent));
    final at = target.indexOf('@');
    if (at < 0) {
      return rejectBare ? [] : [target, 'Server:21116', configuredBareKey];
    }
    if (at > 0 && preferCurrent && matches) {
      if (selectedWebsocket != null) return [target, selectedWebsocket!];
      return [target.substring(0, at), target.substring(at + 1)];
    }
    return [target];
  }
}

List<String> serverConnectionArgs(String id, String server,
        [String key = '']) =>
    [
      '--connect',
      id,
      '--expected-rendezvous',
      jsonEncode({'server': server, 'key': key}),
    ];

void main() {
  late _RendezvousBinding binding;
  setUp(() {
    binding = _RendezvousBinding();
    platformFFI.setBindingForTesting(binding);
  });

  test('request-approval links preserve their connection type', () async {
    for (final type in [
      'connect',
      'play',
      'file-transfer',
      'view-camera',
      'port-forward',
      'rdp',
      'terminal',
      'terminal-admin',
    ]) {
      expect(
        await urlLinkToCmdArgs(
            Uri.parse('rustdesk://$type/123456789?request-approval=true')),
        ['--$type', '123456789', '--request-approval'],
      );
    }
    expect(
      await urlLinkToCmdArgs(
          Uri.parse('rustdesk://123456789?request-approval=true')),
      ['--connect', '123456789', '--request-approval'],
    );
  });

  test('omitted and empty passwords retain automatic login', () async {
    expect(await urlLinkToCmdArgs(Uri.parse('rustdesk://123456789')),
        ['--connect', '123456789']);
    for (final query in ['password=', 'password=&request-approval=false']) {
      expect(await urlLinkToCmdArgs(Uri.parse('rustdesk://123456789?$query')),
          ['--connect', '123456789', '--password', '']);
    }
  });

  test('request-approval ignores an explicit password in either order',
      () async {
    for (final query in [
      'password=provided&request-approval=true',
      'request-approval=true&password=provided',
    ]) {
      expect(await urlLinkToCmdArgs(Uri.parse('rustdesk://123456789?$query')),
          ['--connect', '123456789', '--request-approval']);
    }
  });

  test('connect paths decode the device ID without changing its case',
      () async {
    const id = 'OfficeMünchen/r@Server:21117';
    final uri =
        Uri.parse('rustdesk://connect-id/peer/${Uri.encodeComponent(id)}');
    expect(
        await urlLinkToCmdArgs(uri), serverConnectionArgs(id, 'Server:21117'));
    for (final suffix in ['', '/r']) {
      expect(await urlLinkToCmdArgs(Uri.parse('rustdesk://connect-id$suffix')),
          ['--connect', 'connect-id$suffix']);
    }
    const websocketTarget = 'OfficePC@wss://Server:21117/r';
    expect(
      await urlLinkToCmdArgs(Uri.parse(
          'rustdesk://connect-id/peer/${Uri.encodeComponent(websocketTarget)}')),
      serverConnectionArgs(websocketTarget, 'wss://Server:21117/r'),
    );
    const cliTarget = 'OfficePC@ws://Server:21117/custom%2Froute%20one';
    expect(await urlLinkToCmdArgs(Uri.parse('rustdesk://connect/$cliTarget')),
        ['--connect', cliTarget]);
  });

  test('connect paths reject empty and unsafe decoded device IDs', () async {
    for (final path in [
      '%EF%BB%BF',
      '%EF%BB%BFOfficePC',
      '%20%C2%A0',
      'Office%20PC',
      'Office%C2%A0PC',
      '%2Fr',
      '%5Cr',
      '%2Fr%2Fr',
      '%2Fr%40Server',
      '%40Server',
      '%C2%A0%2Fr',
      'OfficePC%40',
      'OfficePC%40%3Fkey%3Dvalue',
      'OfficePC%40Server%3Fkey%3Dvalue%3Fother',
      '%3Fkey%3Dvalue',
      '%FF',
      'Office%00PC',
      'Office%C2%85PC',
      'Office%E2%80%A8PC',
    ]) {
      expect(
          await urlLinkToCmdArgs(Uri.parse('rustdesk://connect-id/peer/$path')),
          isNull);
    }
  });

  test('connect paths keep the server address and outer key separate',
      () async {
    const id = 'OfficePC/r@Server:21117';
    final encoded = Uri.encodeComponent(id);
    expect(
      await urlLinkToCmdArgs(
          Uri.parse('rustdesk://connect-id/peer/$encoded?key=test%2B%2F%3D')),
      serverConnectionArgs('$id?key=test+/=', 'Server:21117', 'test+/='),
    );
    for (final target in [
      '$id?key=inline',
      '$id?',
      'OfficePC@wss://Server.example/ws/id?region=eu',
      'OfficePC@wss://Server.example/ws/id?key=route',
    ]) {
      expect(
        await urlLinkToCmdArgs(Uri.parse(
            'rustdesk://connect-id/peer/${Uri.encodeComponent(target)}')),
        isNull,
      );
    }
    for (final query in [
      'key=one&key=two',
      'key=one&KEY=two',
      'key=one&%E2%84%AAey=two',
      'key=one%26two',
      'key=one%3Ftwo',
      'key=one%40two',
    ]) {
      expect(
          await urlLinkToCmdArgs(
              Uri.parse('rustdesk://connect-id/peer/$encoded?$query')),
          isNull);
    }
  });

  test('only encoded connections can select the native peer identity',
      () async {
    const target = 'OfficePC/r@Server:21117';
    final link = 'rustdesk://connect-id/peer/${Uri.encodeComponent(target)}';
    expect(
      await urlLinkToCmdArgs(Uri.parse('$link?key=test&prefer_current=1')),
      serverConnectionArgs('$target?key=test', 'Server:21117', 'test'),
    );
    binding.matches = true;
    expect(
      await urlLinkToCmdArgs(Uri.parse('$link?prefer_current=1')),
      [
        '--connect',
        'OfficePC/r',
        '--expected-rendezvous',
        '{"server":"Server:21117","key":""}',
      ],
    );
    expect(
      await urlLinkToCmdArgs(
          Uri.parse('rustdesk://connect-id/peer/OfficePC?prefer_current=1')),
      serverConnectionArgs('OfficePC', 'Server:21116'),
    );
    expect(
      await urlLinkToCmdArgs(
          Uri.parse('rustdesk://connect/$target?key=test&prefer_current=1')),
      ['--connect', '$target?key=test'],
    );
    expect(binding.requests,
        [(target, 'test', true), (target, '', true), ('OfficePC', '', true)]);
  });

  test('the native WSS choice preserves a qualified connection identity',
      () async {
    const target = 'OfficePC@Server:21117';
    binding.matches = true;
    binding.selectedWebsocket = 'wss://server:8443/ws/id';
    final link = 'rustdesk://connect-id/peer/${Uri.encodeComponent(target)}';
    expect(
      await urlLinkToCmdArgs(Uri.parse('$link?key=test&prefer_current=1')),
      [
        '--connect',
        '$target?key=test',
        '--expected-rendezvous',
        '{"server":"wss://server:8443/ws/id","key":"test","fixed_endpoint":true}',
      ],
    );
  });

  test('a bare encoded link retains the native endpoint and public key',
      () async {
    binding.configuredBareKey = 'configured-public-key';
    expect(
      await urlLinkToCmdArgs(Uri.parse('rustdesk://connect-id/peer/OfficePC')),
      serverConnectionArgs('OfficePC', 'Server:21116', 'configured-public-key'),
    );
  });

  test('a rejected dedicated URI never falls back to direct connection',
      () async {
    binding.rejectBare = true;
    expect(
        await urlLinkToCmdArgs(
            Uri.parse('rustdesk://connect-id/peer/192.0.2.1?recent=true')),
        isNull);
    expect(await urlLinkToCmdArgs(Uri.parse('rustdesk://connect/192.0.2.1')),
        ['--connect', '192.0.2.1']);
    expect(binding.requests, [('192.0.2.1', '', false)]);
  });

  test(
      'encoded connection preserves the password and approval takes precedence',
      () async {
    const link = 'rustdesk://connect-id/peer/OfficePC?password=a%2Bb%26c%3Dd';
    expect(await urlLinkToCmdArgs(Uri.parse(link)),
        [
          '--connect', 'OfficePC', '--password', 'a+b&c=d',
          '--expected-rendezvous', '{"server":"Server:21116","key":""}',
        ]);
    expect(await urlLinkToCmdArgs(Uri.parse('$link&request-approval=true')),
        [
          '--connect', 'OfficePC', '--request-approval',
          '--expected-rendezvous', '{"server":"Server:21116","key":""}',
        ]);
  });
}
