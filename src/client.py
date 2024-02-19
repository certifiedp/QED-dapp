import requests
import asyncio
import argparse


def list_proposals(base_url: str):
    response = requests.get(f"{base_url}/")
    print("List of proposals:")
    print(response.text)


def propose(base_url: str, proposer_id: int, statement: str):
    proposal_data = {"proposer_id": proposer_id, "statement": statement}
    response = requests.post(f"{base_url}/propose", json=proposal_data)
    print("Proposal submission response:")
    print(response.text)


def vote(base_url: str, proposal_id: str, voter_id: int, vote: int):
    vote_data = {"proposal_id": proposal_id,
                 "voter_id": voter_id, "is_yes": vote}
    response = requests.post(f"{base_url}/vote", json=vote_data)
    print("Voting response:")
    print(response.text)


def delegate(base_url: str, proposal_id: str, voter_id: int, delegator_id: int):
    delegate_data = {"proposal_id": proposal_id,
                     "voter_id": voter_id, "delegator_id": delegator_id}
    response = requests.post(f"{base_url}/delegate", json=delegate_data)
    print("Delegation response:")
    print(response.text)


def finalize(base_url: str, proposal_id: str, finalizer_id: int):
    finalize_data = {"proposal_id": proposal_id, 'finalizer_id': finalizer_id}
    response = requests.post(f"{base_url}/finalize", json=finalize_data)
    print("Finalize response:")
    print(response.text)


BASE_URL = "http://127.0.0.1:8080"

parser = argparse.ArgumentParser(
    prog='VotingClient',)
subparsers = parser.add_subparsers(dest='method', help='Subcommand help')

parser_vote = subparsers.add_parser('vote', help='vote help')
parser_vote.add_argument('proposal_id', type=str)
parser_vote.add_argument('voter_id', type=int)
parser_vote.add_argument('vote', type=int)

parser_delegate = subparsers.add_parser('delegate', help='delegate help')
parser_delegate.add_argument('proposal_id', type=str)
parser_delegate.add_argument('voter_id', type=int)
parser_delegate.add_argument('delegator_id', type=int)

parser_list_proposals = subparsers.add_parser('list')

parser_propose = subparsers.add_parser('propose', help='propose help')
parser_propose.add_argument('proposer_id', type=int)
parser_propose.add_argument('statement', type=str)

parser_finalize = subparsers.add_parser('finalize', help='finalize help')
parser_finalize.add_argument('proposal_id', type=str)
parser_finalize.add_argument('finalizer_id', type=int)


args = parser.parse_args()
if args.method == 'vote':
    vote(BASE_URL, args.proposal_id, args.voter_id, bool(args.vote))
elif args.method == 'propose':
    propose(BASE_URL, args.proposer_id, args.statement)
elif args.method == 'finalize':
    finalize(BASE_URL, args.proposal_id, args.finalizer_id)
elif args.method == 'list':
    list_proposals(BASE_URL)
elif args.method == 'delegate':
    delegate(BASE_URL, args.proposal_id, args.voter_id, args.delegator_id)
